use cid::Cid;
use export_macro::vm_test;
use fil_actor_reward::{
    AwardBlockRewardParams, CancelPendingParams, ClaimParams, ClaimReturn, DENOM, DistributionInit,
    ExplicitDistribution, Method as RewardMethod, PendingWrite, PendingWriteOp, RecipientShare,
    RecipientTable, RegisterStreamParams, RegisterStreamPayload, RemoveStreamParams,
    SetDistributionParams, SetDistributionPayload, SetWeightRecordsParams, State as RewardState,
    Stream, StreamAccrual, StreamsState, WeightRecord, WeightRecordUpdate,
};
use fil_actors_runtime::{
    BURNT_FUNDS_ACTOR_ADDR, EventBuilder, REWARD_ACTOR_ADDR, REWARD_ACTOR_ID, SYSTEM_ACTOR_ADDR,
    runtime::Policy,
};
use fvm_ipld_encoding::{CborStore, RawBytes};
use fvm_shared::{address::Address, econ::TokenAmount, sector::RegisteredPoStProof};
use multihash_codetable::Code;
use num_traits::Zero;
use vm_api::{
    VM,
    trace::EmittedEvent,
    util::{DynBlockstore, apply_ok, apply_ok_implicit, get_state, mutate_state},
};

use crate::util::{assert_invariants, create_accounts, create_miner};

const CONSENSUS_STREAM_ID: u64 = 1;
const SERVICE_STREAM_ID: u64 = 2;

fn weight(percent: u64) -> WeightRecord {
    let value = DENOM / 100 * percent;
    WeightRecord { v_start: value, slope: 0, t_start: 0, floor: value, cap: value }
}

fn install_stream_state(v: &dyn VM, swa: Address, writer: Address, recipient: Address) -> Cid {
    let streams = StreamsState {
        streams: vec![
            Stream { id: CONSENSUS_STREAM_ID, weight: weight(25), distribution: None },
            Stream {
                id: SERVICE_STREAM_ID,
                weight: weight(25),
                distribution: Some(ExplicitDistribution {
                    writer,
                    shares: vec![RecipientShare { recipient, share: DENOM }],
                    payable: RecipientTable::default(),
                    claimed_period: RecipientTable::default(),
                }),
            },
        ],
        ..Default::default()
    };
    let store = DynBlockstore::wrap(v.blockstore());
    let root = store.put_cbor(&streams, Code::Blake2b256).unwrap();
    mutate_state(v, &REWARD_ACTOR_ADDR, |state: &mut RewardState| {
        state.this_epoch_reward = TokenAmount::from_atto(20);
        state.total_minted_reward = TokenAmount::zero();
        state.total_burn_minted = TokenAmount::zero();
        state.total_explicit_minted = TokenAmount::zero();
        state.accrued = vec![StreamAccrual { id: SERVICE_STREAM_ID, amount: TokenAmount::zero() }];
        state.swa_timelock_epochs = 2;
        state.swa_actor = swa;
        state.streams_root = root;
    });
    root
}

fn load_reward_state(v: &dyn VM) -> (RewardState, StreamsState) {
    let state: RewardState = get_state(v, &REWARD_ACTOR_ADDR).unwrap();
    let streams =
        DynBlockstore::wrap(v.blockstore()).get_cbor(&state.streams_root).unwrap().unwrap();
    (state, streams)
}

fn write_event(kind: &'static str, write: &PendingWrite, include_payload: bool) -> EmittedEvent {
    let mut event = EventBuilder::new()
        .typ(kind)
        .field_indexed("op", &write.op)
        .field("effective-epoch", &write.effective_epoch);
    if let Some(id) = write.id {
        event = event.field_indexed("stream-id", &id);
    }
    if include_payload {
        event = event.field("payload", &write.payload);
    }
    EmittedEvent { emitter: REWARD_ACTOR_ID, event: event.build().unwrap() }
}

fn claim_event(recipient: Address, amount: &TokenAmount) -> EmittedEvent {
    EmittedEvent {
        emitter: REWARD_ACTOR_ID,
        event: EventBuilder::new()
            .typ("claim-payout")
            .field_indexed("stream-id", &SERVICE_STREAM_ID)
            .field_indexed("recipient", &recipient.id().unwrap())
            .field("amount", amount)
            .build()
            .unwrap(),
    }
}

fn take_last_events(v: &dyn VM) -> Vec<EmittedEvent> {
    v.take_invocations().pop().expect("missing invocation trace").events
}

#[vm_test]
pub fn reward_f02_award_and_claim(v: &dyn VM) {
    let accounts = create_accounts(v, 3, &TokenAmount::from_whole(10_000));
    let owner = accounts[0];
    let swa = accounts[1];
    let recipient = accounts[2];
    let (miner, _) = create_miner(
        v,
        &owner,
        &owner,
        RegisteredPoStProof::StackedDRGWindow32GiBV1P1,
        &TokenAmount::zero(),
    );
    let streams_root = install_stream_state(v, swa, owner, recipient);

    let miner_before = v.balance(&miner);
    let burn_before = v.balance(&BURNT_FUNDS_ACTOR_ADDR);
    let reward_before = v.balance(&REWARD_ACTOR_ADDR);
    apply_ok_implicit(
        v,
        &SYSTEM_ACTOR_ADDR,
        &REWARD_ACTOR_ADDR,
        &TokenAmount::zero(),
        RewardMethod::AwardBlockReward as u64,
        Some(AwardBlockRewardParams {
            miner,
            penalty: TokenAmount::zero(),
            gas_reward: TokenAmount::zero(),
            win_count: 1,
        }),
    );

    let miner_reward = TokenAmount::from_atto(1);
    let service_reward = TokenAmount::from_atto(1);
    let burn = TokenAmount::from_atto(2);
    assert_eq!(miner_before + &miner_reward, v.balance(&miner));
    assert_eq!(burn_before + &burn, v.balance(&BURNT_FUNDS_ACTOR_ADDR));
    assert_eq!(&reward_before - &miner_reward - &burn, v.balance(&REWARD_ACTOR_ADDR));

    let (state, streams) = load_reward_state(v);
    assert_eq!(streams_root, state.streams_root);
    assert_eq!(TokenAmount::from_atto(4), state.total_minted_reward);
    assert_eq!(burn, state.total_burn_minted);
    assert_eq!(service_reward, state.total_explicit_minted);
    assert_eq!(service_reward, state.accrued[0].amount);
    assert!(streams.pending_writes.is_empty());
    assert_invariants(v, &Policy::default(), None);

    let removal = PendingWrite {
        id: Some(SERVICE_STREAM_ID),
        op: PendingWriteOp::RemoveStream,
        payload: RawBytes::new(vec![0x80]),
        effective_epoch: 2,
    };
    apply_ok(
        v,
        &swa,
        &REWARD_ACTOR_ADDR,
        &TokenAmount::zero(),
        RewardMethod::RemoveStreamExported as u64,
        Some(RemoveStreamParams { id: SERVICE_STREAM_ID }),
    );
    assert_eq!(vec![write_event("write-queued", &removal, true)], take_last_events(v));

    v.set_epoch(2);
    mutate_state(v, &REWARD_ACTOR_ADDR, |state: &mut RewardState| state.epoch = 2);

    let recipient_before = v.balance(&recipient);
    let unresolved = Address::new_bls(&[42; fvm_shared::address::BLS_PUB_LEN]).unwrap();
    let claimed: ClaimReturn = apply_ok(
        v,
        &swa,
        &REWARD_ACTOR_ADDR,
        &TokenAmount::zero(),
        RewardMethod::ClaimExported as u64,
        Some(ClaimParams { id: SERVICE_STREAM_ID, wallets: vec![recipient, unresolved] }),
    )
    .deserialize()
    .unwrap();
    assert_eq!(vec![service_reward.clone(), TokenAmount::zero()], claimed.amounts);
    assert_eq!(recipient_before + &service_reward, v.balance(&recipient));
    assert_eq!(
        vec![
            write_event("write-applied", &removal, false),
            claim_event(recipient, &service_reward),
        ],
        take_last_events(v)
    );
    let (state, streams) = load_reward_state(v);
    assert!(state.accrued.is_empty());
    assert_eq!(vec![CONSENSUS_STREAM_ID], streams.streams.iter().map(|s| s.id).collect::<Vec<_>>());
    assert!(streams.tombstones.is_empty());

    let reward_after_claim = v.balance(&REWARD_ACTOR_ADDR);
    let claimed_again: ClaimReturn = apply_ok(
        v,
        &owner,
        &REWARD_ACTOR_ADDR,
        &TokenAmount::zero(),
        RewardMethod::ClaimExported as u64,
        Some(ClaimParams { id: SERVICE_STREAM_ID, wallets: vec![recipient] }),
    )
    .deserialize()
    .unwrap();
    assert_eq!(vec![TokenAmount::zero()], claimed_again.amounts);
    assert_eq!(reward_after_claim, v.balance(&REWARD_ACTOR_ADDR));
    assert!(take_last_events(v).is_empty());
    assert_invariants(v, &Policy::default(), None);
}

#[vm_test]
pub fn reward_f02_queued_apply_and_drop(v: &dyn VM) {
    let accounts = create_accounts(v, 4, &TokenAmount::from_whole(10_000));
    let swa = accounts[0];
    let writer = accounts[1];
    let recipient = accounts[2];
    let next_writer = accounts[3];
    install_stream_state(v, swa, writer, recipient);

    let distribution =
        DistributionInit { writer, shares: vec![RecipientShare { recipient, share: DENOM }] };
    let register_params = RegisterStreamParams {
        id: 3,
        weight: WeightRecord::default(),
        distribution: Some(distribution.clone()),
        activation_epoch: 2,
    };
    let registration = PendingWrite {
        id: Some(3),
        op: PendingWriteOp::RegisterStream,
        payload: RawBytes::serialize(&RegisterStreamPayload {
            weight: WeightRecord::default(),
            distribution: Some(distribution),
        })
        .unwrap(),
        effective_epoch: 2,
    };
    apply_ok(
        v,
        &swa,
        &REWARD_ACTOR_ADDR,
        &TokenAmount::zero(),
        RewardMethod::RegisterStreamExported as u64,
        Some(register_params),
    );
    assert_eq!(vec![write_event("write-queued", &registration, true)], take_last_events(v));

    let weight_params =
        SetWeightRecordsParams { updates: vec![WeightRecordUpdate { id: 3, weight: weight(10) }] };
    let stranded = PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: RawBytes::serialize(&weight_params).unwrap(),
        effective_epoch: 2,
    };
    apply_ok(
        v,
        &swa,
        &REWARD_ACTOR_ADDR,
        &TokenAmount::zero(),
        RewardMethod::SetWeightRecordsExported as u64,
        Some(weight_params),
    );
    assert_eq!(vec![write_event("write-queued", &stranded, true)], take_last_events(v));

    let distribution_params = SetDistributionParams { id: SERVICE_STREAM_ID, writer: next_writer };
    let distribution_write = PendingWrite {
        id: Some(SERVICE_STREAM_ID),
        op: PendingWriteOp::SetDistribution,
        payload: RawBytes::serialize(&SetDistributionPayload { writer: next_writer }).unwrap(),
        effective_epoch: 2,
    };
    apply_ok(
        v,
        &swa,
        &REWARD_ACTOR_ADDR,
        &TokenAmount::zero(),
        RewardMethod::SetDistributionExported as u64,
        Some(distribution_params),
    );
    assert_eq!(vec![write_event("write-queued", &distribution_write, true)], take_last_events(v));

    v.set_epoch(1);
    apply_ok(
        v,
        &swa,
        &REWARD_ACTOR_ADDR,
        &TokenAmount::zero(),
        RewardMethod::CancelPendingExported as u64,
        Some(CancelPendingParams { id: Some(3), op: PendingWriteOp::RegisterStream }),
    );
    assert_eq!(vec![write_event("write-cancelled", &registration, false)], take_last_events(v));

    let (_, queued) = load_reward_state(v);
    assert_eq!(vec![stranded.clone(), distribution_write.clone()], queued.pending_writes);

    v.set_epoch(2);
    mutate_state(v, &REWARD_ACTOR_ADDR, |state: &mut RewardState| state.epoch = 2);
    let result: ClaimReturn = apply_ok(
        v,
        &writer,
        &REWARD_ACTOR_ADDR,
        &TokenAmount::zero(),
        RewardMethod::ClaimExported as u64,
        Some(ClaimParams { id: 999, wallets: Vec::new() }),
    )
    .deserialize()
    .unwrap();
    assert!(result.amounts.is_empty());
    assert_eq!(
        vec![
            write_event("write-applied", &distribution_write, false),
            write_event("write-dropped", &stranded, false),
        ],
        take_last_events(v)
    );

    let (state, streams) = load_reward_state(v);
    assert!(streams.pending_writes.is_empty());
    assert_eq!(next_writer, streams.streams[1].distribution.as_ref().unwrap().writer);
    assert_eq!(TokenAmount::zero(), state.total_minted_reward);
    assert_eq!(TokenAmount::zero(), state.total_burn_minted);
    assert_eq!(TokenAmount::zero(), state.total_explicit_minted);
    assert_eq!(TokenAmount::zero(), state.accrued[0].amount);
    assert_invariants(v, &Policy::default(), None);
}
