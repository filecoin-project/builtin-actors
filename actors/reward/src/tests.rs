use fil_actors_runtime::REWARD_ACTOR_ADDR;
use fil_actors_runtime::test_utils::{
    ACCOUNT_ACTOR_CODE_ID, EVM_ACTOR_CODE_ID, MockRuntime, SYSTEM_ACTOR_CODE_ID,
};
use fvm_ipld_encoding::{CborStore, RawBytes};
use multihash_codetable::Code;

use super::*;

const WRITER: u64 = 200;
const RECIPIENT: u64 = 201;

fn weight(value: u64) -> WeightRecord {
    WeightRecord { v_start: value, slope: 0, t_start: 0, floor: value, cap: value }
}

/// One implicit stream at 60%, one explicit at 20% paying a single recipient, and a due
/// weight write so the streams block is dirty by the time the invariants run.
fn base_runtime() -> (MockRuntime, PendingWrite) {
    let rt = MockRuntime { receiver: REWARD_ACTOR_ADDR, ..Default::default() };
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.set_address_actor_type(Address::new_id(RECIPIENT), *EVM_ACTOR_CODE_ID);
    rt.set_address_actor_type(BURNT_FUNDS_ACTOR_ADDR, *ACCOUNT_ACTOR_CODE_ID);
    let pending = PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: RawBytes::serialize(&SetWeightRecordsParams {
            updates: vec![WeightRecordUpdate { id: 1, weight: weight(DENOM / 100 * 60) }],
        })
        .unwrap(),
        effective_epoch: 0,
    };
    let streams = StreamsState {
        streams: vec![
            Stream { id: 1, weight: weight(DENOM / 100 * 60), distribution: None },
            Stream {
                id: 2,
                weight: weight(DENOM / 100 * 20),
                distribution: Some(ExplicitDistribution {
                    writer: Address::new_id(WRITER),
                    shares: vec![RecipientShare {
                        recipient: Address::new_id(RECIPIENT),
                        share: DENOM,
                    }],
                    payable: RecipientTable::default(),
                    claimed_period: RecipientTable::default(),
                }),
            },
        ],
        pending_writes_queue: vec![pending.clone()],
        ..Default::default()
    };
    let streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    rt.replace_state(&State {
        accrued: vec![StreamAccrual { id: 2, amount: TokenAmount::zero() }],
        this_epoch_reward: TokenAmount::from_atto(25),
        swa_timelock_epochs: 2,
        swa_actor: Address::new_id(1001),
        streams_root,
        ..Default::default()
    });
    rt.set_balance(TokenAmount::from_atto(100));
    (rt, pending)
}

fn queued_writes(rt: &MockRuntime) -> Vec<PendingWrite> {
    let state: State = rt.get_state();
    let streams: StreamsState = rt.store.get_cbor(&state.streams_root).unwrap().unwrap();
    streams.pending_writes_queue
}

#[test]
fn a_mutation_that_breaks_the_invariants_aborts_and_stores_nothing() {
    let (rt, pending) = base_runtime();
    let before: State = rt.get_state();
    // Driving the shared body directly, so the runtime needs the flag a dispatch would set.
    rt.in_call.replace(true);

    let error = run_mutation(&rt, |ledger, _, _| {
        ledger.corrupt();
        Ok(())
    })
    .expect_err("a corrupt ledger is illegal state");

    assert_eq!(ExitCode::USR_ILLEGAL_STATE, error.exit_code());
    assert!(
        error.msg().contains("accrual IDs do not match live explicit streams"),
        "{}",
        error.msg()
    );
    let after: State = rt.get_state();
    assert_eq!(before.streams_root, after.streams_root);
    assert_eq!(before.accrued, after.accrued);
    // The transaction discards the due write with everything else, so it stays queued.
    assert_eq!(vec![pending], queued_writes(&rt));
}

#[test]
fn an_award_that_breaks_the_invariants_pays_gas_only() {
    let (rt, pending) = base_runtime();
    let before: State = rt.get_state();
    let gas = TokenAmount::from_atto(2);

    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    rt.expect_send_simple(
        Address::new_id(RECIPIENT),
        ext::miner::APPLY_REWARDS_METHOD,
        IpldBlock::serialize_cbor(&ext::miner::ApplyRewardParams {
            reward: gas.clone(),
            penalty: TokenAmount::zero(),
        })
        .unwrap(),
        gas.clone(),
        None,
        ExitCode::OK,
    );
    Ledger::arm_award_fault();
    let params = AwardBlockRewardParams {
        miner: Address::new_id(RECIPIENT),
        penalty: TokenAmount::zero(),
        gas_reward: gas,
        win_count: 1,
    };
    rt.call::<Actor>(Method::AwardBlockReward as u64, IpldBlock::serialize_cbor(&params).unwrap())
        .unwrap();
    rt.verify();

    let after: State = rt.get_state();
    assert_eq!(before.streams_root, after.streams_root);
    assert_eq!(before.accrued, after.accrued);
    assert_eq!(before.total_minted_reward, after.total_minted_reward);
    assert_eq!(before.total_burn_minted, after.total_burn_minted);
    assert_eq!(before.total_explicit_minted, after.total_explicit_minted);
    assert_eq!(vec![pending], queued_writes(&rt));
    assert_eq!(TokenAmount::from_atto(98), *rt.balance.borrow());
}
