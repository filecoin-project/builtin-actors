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

mod this_epoch_reward_tests {
    use super::*;

    fn query(rt: &MockRuntime) -> Result<ThisEpochRewardReturn, ActorError> {
        rt.expect_validate_caller_any();
        let result = rt.call::<Actor>(Method::ThisEpochReward as u64, None);
        rt.verify();
        result.map(|response| response.unwrap().deserialize().unwrap())
    }

    fn save_streams(rt: &MockRuntime, state: &mut State, streams: &StreamsState) {
        state.streams_root = rt.store.put_cbor(streams, Code::Blake2b256).unwrap();
        rt.replace_state(state);
    }

    #[test]
    fn uses_call_epoch_without_applying_due_writes_or_future_checks() {
        let (rt, _) = base_runtime();
        let mut state: State = rt.get_state();
        let mut streams: StreamsState = rt.store.get_cbor(&state.streams_root).unwrap().unwrap();
        let mut implicit = streams.streams.remove(0);
        implicit.id = 3;
        implicit.weight = WeightRecord {
            v_start: DENOM / 100 * 60,
            slope: (DENOM / 100) as i64,
            t_start: 0,
            floor: 0,
            cap: DENOM,
        };
        streams.streams.push(implicit);
        streams.pending_writes_queue[0].payload = RawBytes::serialize(&SetWeightRecordsParams {
            updates: vec![WeightRecordUpdate { id: 3, weight: weight(DENOM / 100 * 40) }],
        })
        .unwrap();
        save_streams(&rt, &mut state, &streams);
        rt.epoch.replace(5);

        let response = query(&rt).unwrap();
        assert_eq!(DENOM / 100 * 65, response.this_epoch_implicit_stream_weight);
        assert_eq!(state.this_epoch_baseline_power, response.this_epoch_baseline_power);
        assert_eq!(state.this_epoch_reward_smoothed, response.this_epoch_reward_smoothed);
        let after: State = rt.get_state();
        assert_eq!(
            fvm_ipld_encoding::to_vec(&state).unwrap(),
            fvm_ipld_encoding::to_vec(&after).unwrap(),
        );
    }

    #[test]
    fn returns_zero_when_implicit_stream_is_absent_or_zero() {
        for absent in [false, true] {
            let (rt, _) = base_runtime();
            let mut state: State = rt.get_state();
            let mut streams: StreamsState =
                rt.store.get_cbor(&state.streams_root).unwrap().unwrap();
            streams.pending_writes_queue.clear();
            if absent {
                streams.streams.remove(0);
            } else {
                streams.streams[0].weight = weight(0);
            }
            save_streams(&rt, &mut state, &streams);
            assert_eq!(0, query(&rt).unwrap().this_epoch_implicit_stream_weight);
        }
    }

    #[test]
    fn returns_zero_for_invalid_stored_records_or_current_total() {
        for case in ["implicit weight", "explicit weight", "total", "recipient", "accrual"] {
            let (rt, _) = base_runtime();
            let mut state: State = rt.get_state();
            let mut streams: StreamsState =
                rt.store.get_cbor(&state.streams_root).unwrap().unwrap();
            match case {
                "implicit weight" => streams.streams[0].weight.cap = DENOM + 1,
                "explicit weight" => streams.streams[1].weight.cap = DENOM + 1,
                "total" => streams.streams[0].weight = weight(DENOM / 100 * 90),
                "recipient" => {
                    streams.streams[1].distribution.as_mut().unwrap().shares[0].share = DENOM + 1;
                }
                "accrual" => state.accrued.clear(),
                _ => unreachable!(),
            }
            save_streams(&rt, &mut state, &streams);
            assert_eq!(0, query(&rt).unwrap().this_epoch_implicit_stream_weight, "{case}");
        }
    }

    #[test]
    fn returns_zero_for_undecodable_streams() {
        let (rt, _) = base_runtime();
        let mut state: State = rt.get_state();
        state.streams_root = rt.store.put_cbor(&0u64, Code::Blake2b256).unwrap();
        rt.replace_state(&state);
        assert_eq!(0, query(&rt).unwrap().this_epoch_implicit_stream_weight);
    }

    #[test]
    fn aborts_when_streams_block_is_missing() {
        let (rt, _) = base_runtime();
        let mut state: State = rt.get_state();
        state.streams_root = Cid::default();
        rt.replace_state(&state);
        assert_eq!(ExitCode::USR_ILLEGAL_STATE, query(&rt).unwrap_err().exit_code());
    }
}
