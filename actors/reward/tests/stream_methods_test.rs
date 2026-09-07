use std::cell::RefCell;

use fil_actor_reward::testing::check_state_invariants;
use fil_actor_reward::{
    Actor as RewardActor, AwardBlockRewardParams, CancelPendingParams, DENOM, DistributionInit,
    ExplicitDistribution, MAX_RECIPIENTS, MAX_STREAMS, Method, PENALTY_MULTIPLIER, PendingWrite,
    PendingWriteOp, RecipientShare, RegisterStreamParams, RegisterStreamPayload,
    RemoveStreamParams, SetDistributionParams, SetDistributionPayload, SetSharesParams,
    SetWeightRecordsParams, State, Stream, StreamsState, WeightRecord, WeightRecordUpdate, ext,
};
use fil_actors_runtime::test_utils::{
    ACCOUNT_ACTOR_CODE_ID, EVM_ACTOR_CODE_ID, MockRuntime, SYSTEM_ACTOR_CODE_ID, expect_abort,
};
use fil_actors_runtime::{
    BURNT_FUNDS_ACTOR_ADDR, EventBuilder, REWARD_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_encoding::{CBOR, CborStore, RawBytes, ipld_block::IpldBlock};
use fvm_shared::METHOD_SEND;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::{ErrorNumber, ExitCode};
use fvm_shared::sys::SendFlags;
use multihash_codetable::{Code, MultihashDigest};
use num_traits::Zero;

const SWA_ACTOR_ID: u64 = 1001;
const WRITER: u64 = 200;
const RECIPIENT_A: u64 = 201;
const RECIPIENT_B: u64 = 202;
const MINER: u64 = 203;
const NEXT_WRITER: u64 = 204;
/// The first of the 512 recipients a full stream table pays.
const FIRST_BOUND_RECIPIENT: u64 = 2_000;

fn swa_actor() -> Address {
    Address::new_id(SWA_ACTOR_ID)
}

fn weight(value: u64) -> WeightRecord {
    WeightRecord { v_start: value, slope: 0, t_start: 0, floor: value, cap: value }
}

fn pct(value: u64) -> u64 {
    DENOM / 100 * value
}

fn base_runtime() -> MockRuntime {
    let rt = MockRuntime {
        receiver: REWARD_ACTOR_ADDR,
        caller: RefCell::new(swa_actor()),
        caller_type: RefCell::new(*EVM_ACTOR_CODE_ID),
        ..Default::default()
    };
    for id in [SWA_ACTOR_ID, WRITER, RECIPIENT_A, RECIPIENT_B, MINER] {
        rt.set_address_actor_type(Address::new_id(id), *EVM_ACTOR_CODE_ID);
    }
    rt.set_address_actor_type(BURNT_FUNDS_ACTOR_ADDR, *ACCOUNT_ACTOR_CODE_ID);
    let streams = StreamsState {
        streams: vec![
            Stream { id: 1, weight: weight(pct(60)), distribution: None },
            Stream {
                id: 2,
                weight: weight(pct(20)),
                distribution: Some(ExplicitDistribution {
                    writer: Address::new_id(WRITER),
                    shares: vec![RecipientShare {
                        recipient: Address::new_id(RECIPIENT_A),
                        share: DENOM,
                    }],
                }),
            },
        ],
        ..Default::default()
    };
    let streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    rt.replace_state(&State {
        epoch: 0,
        swa_timelock_epochs: 2,
        swa_actor: swa_actor(),
        streams_root,
        ..Default::default()
    });
    rt
}

fn load_streams(rt: &MockRuntime) -> StreamsState {
    let state: State = rt.get_state();
    rt.store.get_cbor(&state.streams_root).unwrap().unwrap()
}

fn assert_state_invariants(rt: &MockRuntime) {
    let state: State = rt.get_state();
    let balance = rt.balance.borrow();
    let (_, messages) =
        check_state_invariants(&state, &*rt.store, state.epoch - 1, *rt.epoch.borrow(), &balance);
    messages.assert_empty();
}

fn call<P: serde::Serialize>(
    rt: &MockRuntime,
    method: Method,
    params: &P,
) -> Result<Option<IpldBlock>, fil_actors_runtime::ActorError> {
    rt.call::<RewardActor>(method as u64, IpldBlock::serialize_cbor(params).unwrap())
}

fn expect_write_event(rt: &MockRuntime, typ: &str, write: &PendingWrite, queued: bool) {
    let mut event = EventBuilder::new()
        .typ(typ)
        .field_indexed("op", &write.op)
        .field("effective-epoch", &write.effective_epoch);
    if let Some(id) = write.id {
        event = event.field_indexed("stream-id", &id);
    }
    if queued {
        event = event.field("payload", &write.payload);
    }
    rt.expect_emitted_event(event.build().unwrap());
}

/// One recipient's payout from one award, which the actor sends with method 0.
fn expect_payout(rt: &MockRuntime, recipient: u64, amount: u64, exit_code: ExitCode) {
    rt.expect_send_simple(
        Address::new_id(recipient),
        METHOD_SEND,
        None,
        TokenAmount::from_atto(amount),
        None,
        exit_code,
    );
}

fn expect_miner_reward(
    rt: &MockRuntime,
    reward: TokenAmount,
    penalty: TokenAmount,
    exit_code: ExitCode,
) {
    rt.expect_send_simple(
        Address::new_id(MINER),
        ext::miner::APPLY_REWARDS_METHOD,
        IpldBlock::serialize_cbor(&ext::miner::ApplyRewardParams {
            reward: reward.clone(),
            penalty: &penalty * PENALTY_MULTIPLIER,
        })
        .unwrap(),
        reward,
        None,
        exit_code,
    );
}

fn expect_burn(rt: &MockRuntime, amount: TokenAmount, exit_code: ExitCode) {
    rt.expect_send_simple(BURNT_FUNDS_ACTOR_ADDR, METHOD_SEND, None, amount, None, exit_code);
}

fn award(
    rt: &MockRuntime,
    gas_reward: TokenAmount,
    penalty: TokenAmount,
    win_count: i64,
) -> Result<Option<IpldBlock>, fil_actors_runtime::ActorError> {
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    call(
        rt,
        Method::AwardBlockReward,
        &AwardBlockRewardParams { miner: Address::new_id(MINER), penalty, gas_reward, win_count },
    )
}

#[test]
fn enforces_caller_boundaries_and_existing_share_recipients() {
    let rt = base_runtime();
    let weight_params = SetWeightRecordsParams { updates: Vec::new() };
    let requests = [
        (Method::SetWeightRecordsExported, IpldBlock::serialize_cbor(&weight_params).unwrap()),
        (Method::StepWeightRecordsExported, IpldBlock::serialize_cbor(&weight_params).unwrap()),
        (
            Method::RegisterStreamExported,
            IpldBlock::serialize_cbor(&RegisterStreamParams {
                id: 3,
                weight: weight(pct(10)),
                distribution: None,
                activation_epoch: 2,
            })
            .unwrap(),
        ),
        (
            Method::RemoveStreamExported,
            IpldBlock::serialize_cbor(&RemoveStreamParams { id: 2 }).unwrap(),
        ),
        (
            Method::SetDistributionExported,
            IpldBlock::serialize_cbor(&SetDistributionParams {
                id: 2,
                writer: Address::new_id(WRITER),
            })
            .unwrap(),
        ),
        (
            Method::CancelPendingExported,
            IpldBlock::serialize_cbor(&CancelPendingParams {
                id: None,
                op: PendingWriteOp::SetWeightRecords,
            })
            .unwrap(),
        ),
    ];

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(999));
    for (method, params) in requests {
        rt.expect_validate_caller_addr(vec![swa_actor()]);
        expect_abort(ExitCode::USR_FORBIDDEN, rt.call::<RewardActor>(method as u64, params));
        rt.verify();
    }

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(WRITER));
    rt.expect_validate_caller_any();
    expect_abort(
        ExitCode::USR_NOT_FOUND,
        call(
            &rt,
            Method::SetSharesExported,
            &SetSharesParams {
                id: 2,
                shares: vec![RecipientShare { recipient: Address::new_id(999_999), share: DENOM }],
            },
        ),
    );
    rt.verify();
}

#[test]
fn authorizes_the_swa_address_stored_in_state() {
    let rt = base_runtime();
    let configured = Address::new_id(333);
    let mut state: State = rt.get_state();
    state.swa_actor = configured;
    rt.replace_state(&state);
    rt.set_address_actor_type(configured, *EVM_ACTOR_CODE_ID);

    rt.set_caller(*EVM_ACTOR_CODE_ID, swa_actor());
    rt.expect_validate_caller_addr(vec![configured]);
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        call(
            &rt,
            Method::CancelPendingExported,
            &CancelPendingParams { id: None, op: PendingWriteOp::SetWeightRecords },
        ),
    );
    rt.verify();

    rt.set_caller(*EVM_ACTOR_CODE_ID, configured);
    rt.expect_validate_caller_addr(vec![configured]);
    call(
        &rt,
        Method::CancelPendingExported,
        &CancelPendingParams { id: None, op: PendingWriteOp::SetWeightRecords },
    )
    .unwrap();
    rt.verify();
}

#[test]
fn bounds_recipient_inputs_before_lookup_and_authorization_work() {
    let rt = base_runtime();
    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(999));
    let too_many_shares =
        vec![
            RecipientShare { recipient: Address::new_id(RECIPIENT_A), share: DENOM };
            MAX_RECIPIENTS + 1
        ];
    rt.expect_validate_caller_any();
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        call(&rt, Method::SetSharesExported, &SetSharesParams { id: 2, shares: too_many_shares }),
    );
    rt.verify();

    rt.expect_validate_caller_any();
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        call(
            &rt,
            Method::SetSharesExported,
            &SetSharesParams {
                id: 2,
                shares: vec![RecipientShare { recipient: Address::new_id(999_999), share: DENOM }],
            },
        ),
    );
    rt.verify();
}

#[test]
fn rejects_invalid_method_parameters_without_state_changes() {
    let rt = base_runtime();
    let before = load_streams(&rt);

    macro_rules! swa_reject {
        ($method:expr, $params:expr) => {{
            rt.expect_validate_caller_addr(vec![swa_actor()]);
            expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, call(&rt, $method, &$params));
            rt.verify();
        }};
    }

    swa_reject!(Method::SetWeightRecordsExported, SetWeightRecordsParams { updates: Vec::new() });
    swa_reject!(Method::StepWeightRecordsExported, SetWeightRecordsParams { updates: Vec::new() });
    swa_reject!(
        Method::RegisterStreamExported,
        RegisterStreamParams {
            id: 0,
            weight: weight(pct(10)),
            distribution: None,
            activation_epoch: 2,
        }
    );
    swa_reject!(Method::RemoveStreamExported, RemoveStreamParams { id: 999 });
    swa_reject!(
        Method::SetDistributionExported,
        SetDistributionParams { id: 1, writer: Address::new_id(WRITER) }
    );
    swa_reject!(
        Method::CancelPendingExported,
        CancelPendingParams { id: Some(1), op: PendingWriteOp::SetWeightRecords }
    );
    swa_reject!(
        Method::CancelPendingExported,
        CancelPendingParams { id: None, op: PendingWriteOp::StepWeightRecords }
    );

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(WRITER));
    rt.expect_validate_caller_any();
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        call(&rt, Method::SetSharesExported, &SetSharesParams { id: 2, shares: Vec::new() }),
    );
    rt.verify();

    assert_eq!(before, load_streams(&rt));
}

#[test]
fn mutations_reject_invalid_persisted_state_without_a_due_write() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    // Descending stored share recipients, which no admitted map can produce.
    streams.streams[1].distribution.as_mut().unwrap().shares = vec![
        RecipientShare { recipient: Address::new_id(RECIPIENT_B), share: DENOM / 2 },
        RecipientShare { recipient: Address::new_id(RECIPIENT_A), share: DENOM - DENOM / 2 },
    ];
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    let before = state.clone();
    rt.replace_state(&state);

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(WRITER));
    rt.expect_validate_caller_any();
    expect_abort(
        ExitCode::USR_ILLEGAL_STATE,
        call(
            &rt,
            Method::SetSharesExported,
            &SetSharesParams {
                id: 2,
                shares: vec![RecipientShare {
                    recipient: Address::new_id(RECIPIENT_A),
                    share: DENOM,
                }],
            },
        ),
    );
    rt.verify();

    rt.set_caller(*EVM_ACTOR_CODE_ID, swa_actor());
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_abort(
        ExitCode::USR_ILLEGAL_STATE,
        call(
            &rt,
            Method::SetWeightRecordsExported,
            &SetWeightRecordsParams {
                updates: vec![WeightRecordUpdate { id: 1, weight: weight(pct(60)) }],
            },
        ),
    );
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(before.streams_root, state.streams_root);
    assert!(load_streams(&rt).pending_writes.is_empty());
}

#[test]
fn queue_rejection_has_a_deterministic_exit_code() {
    let rt = base_runtime();
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        call(
            &rt,
            Method::SetWeightRecordsExported,
            &SetWeightRecordsParams { updates: Vec::new() },
        ),
    );
    rt.verify();
    assert!(load_streams(&rt).pending_writes.is_empty());
}

#[test]
fn rejects_malformed_cbor_for_every_exported_method() {
    let rt = base_runtime();
    for method in [
        Method::SetWeightRecordsExported,
        Method::StepWeightRecordsExported,
        Method::RegisterStreamExported,
        Method::RemoveStreamExported,
        Method::SetDistributionExported,
        Method::CancelPendingExported,
        Method::SetSharesExported,
    ] {
        expect_abort(
            ExitCode::USR_SERIALIZATION,
            rt.call::<RewardActor>(
                method as u64,
                Some(IpldBlock { codec: CBOR, data: vec![0xff] }),
            ),
        );
    }
}
#[test]
fn queues_cancels_and_applies_weight_writes() {
    let rt = base_runtime();
    let params = SetWeightRecordsParams {
        updates: vec![
            WeightRecordUpdate { id: 2, weight: weight(pct(25)) },
            WeightRecordUpdate { id: 1, weight: weight(pct(55)) },
        ],
    };
    let payload = RawBytes::serialize(&SetWeightRecordsParams {
        updates: vec![
            WeightRecordUpdate { id: 1, weight: weight(pct(55)) },
            WeightRecordUpdate { id: 2, weight: weight(pct(25)) },
        ],
    })
    .unwrap();
    let queued = PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: payload.clone(),
        effective_epoch: 2,
    };

    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-queued", &queued, true);
    call(&rt, Method::SetWeightRecordsExported, &params).unwrap();
    rt.verify();
    assert_eq!(1, load_streams(&rt).pending_writes.len());

    rt.epoch.replace(1);
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-cancelled", &queued, false);
    call(
        &rt,
        Method::CancelPendingExported,
        &CancelPendingParams { id: None, op: PendingWriteOp::SetWeightRecords },
    )
    .unwrap();
    rt.verify();
    assert!(load_streams(&rt).pending_writes.is_empty());

    let queued = PendingWrite { effective_epoch: 3, ..queued };
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-queued", &queued, true);
    call(&rt, Method::SetWeightRecordsExported, &params).unwrap();
    rt.verify();

    rt.epoch.replace(3);
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-applied", &queued, false);
    // Cancelling an empty slot does nothing of its own, so the due write is all that moves.
    call(
        &rt,
        Method::CancelPendingExported,
        &CancelPendingParams { id: Some(999), op: PendingWriteOp::RemoveStream },
    )
    .unwrap();
    rt.verify();
    let streams = load_streams(&rt);
    assert_eq!(pct(55), streams.streams[0].weight.v_start);
    assert_eq!(pct(25), streams.streams[1].weight.v_start);
    assert!(streams.pending_writes.is_empty());
}

#[test]
fn set_shares_binds_from_the_next_award_and_never_redirects_an_earlier_one() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.this_epoch_reward = TokenAmount::from_atto(500);
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_whole(1_100_000_000));

    // Stream 2 carries 20% of a 100 atto block reward, all of it to recipient A.
    expect_miner_reward(&rt, TokenAmount::from_atto(60), TokenAmount::zero(), ExitCode::OK);
    expect_payout(&rt, RECIPIENT_A, 20, ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(20), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();
    assert_eq!(TokenAmount::from_atto(20), rt.get_state::<State>().total_explicit_minted);

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(WRITER));
    rt.expect_validate_caller_any();
    call(
        &rt,
        Method::SetSharesExported,
        &SetSharesParams {
            id: 2,
            shares: vec![RecipientShare { recipient: Address::new_id(RECIPIENT_B), share: DENOM }],
        },
    )
    .unwrap();
    rt.verify();

    // The next award pays the new map, and nothing goes back to recipient A.
    expect_miner_reward(&rt, TokenAmount::from_atto(60), TokenAmount::zero(), ExitCode::OK);
    expect_payout(&rt, RECIPIENT_B, 20, ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(20), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(200), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(40), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(40), state.total_explicit_minted);
    assert_state_invariants(&rt);
}

#[test]
fn award_burns_a_payout_its_recipient_cannot_receive() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    streams.streams[1].distribution.as_mut().unwrap().shares = vec![
        RecipientShare { recipient: Address::new_id(RECIPIENT_A), share: DENOM / 2 },
        RecipientShare { recipient: Address::new_id(RECIPIENT_B), share: DENOM - DENOM / 2 },
    ];
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(500);
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_whole(1_100_000_000));

    // Recipient A is gone, so its 10 attos join the 20 atto residual in the single burn send.
    // A is the first payout of the loop, so recipient B is paid after the failure.
    expect_miner_reward(&rt, TokenAmount::from_atto(60), TokenAmount::zero(), ExitCode::OK);
    rt.expect_send(
        Address::new_id(RECIPIENT_A),
        METHOD_SEND,
        None,
        TokenAmount::from_atto(10),
        None,
        SendFlags::default(),
        None,
        ExitCode::OK,
        Some(ErrorNumber::NotFound),
    );
    expect_payout(&rt, RECIPIENT_B, 10, ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(30), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(100), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(20), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(20), state.total_explicit_minted);
    assert_state_invariants(&rt);
}

#[test]
fn award_pays_miner_payouts_and_burn_within_the_balance() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    rt.replace_state(&state);
    // Exactly the gas reward plus the block reward, so every atto leaves in this one award.
    rt.set_balance(TokenAmount::from_atto(7));

    expect_miner_reward(&rt, TokenAmount::from_atto(5), TokenAmount::zero(), ExitCode::OK);
    expect_payout(&rt, RECIPIENT_A, 1, ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(1), ExitCode::OK);
    award(&rt, TokenAmount::from_atto(2), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(5), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(1), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(1), state.total_explicit_minted);
    assert_eq!(TokenAmount::zero(), *rt.balance.borrow());
}

#[test]
fn rejects_the_reward_actor_as_a_share_recipient() {
    let rt = base_runtime();
    rt.set_address_actor_type(REWARD_ACTOR_ADDR, *EVM_ACTOR_CODE_ID);

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(WRITER));
    rt.expect_validate_caller_any();
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        call(
            &rt,
            Method::SetSharesExported,
            &SetSharesParams {
                id: 2,
                shares: vec![RecipientShare { recipient: REWARD_ACTOR_ADDR, share: DENOM }],
            },
        ),
    );
    rt.verify();

    rt.set_caller(*EVM_ACTOR_CODE_ID, swa_actor());
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        call(
            &rt,
            Method::RegisterStreamExported,
            &RegisterStreamParams {
                id: 3,
                weight: weight(pct(10)),
                distribution: Some(DistributionInit {
                    writer: Address::new_id(WRITER),
                    shares: vec![RecipientShare { recipient: REWARD_ACTOR_ADDR, share: DENOM }],
                }),
                activation_epoch: 2,
            },
        ),
    );
    rt.verify();
    assert!(load_streams(&rt).pending_writes.is_empty());
}

#[test]
fn set_shares_remains_live_while_the_weight_envelope_is_invalid() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    streams.streams[0].weight = weight(pct(90));
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    rt.replace_state(&state);

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(WRITER));
    rt.expect_validate_caller_any();
    call(
        &rt,
        Method::SetSharesExported,
        &SetSharesParams {
            id: 2,
            shares: vec![RecipientShare { recipient: Address::new_id(RECIPIENT_B), share: DENOM }],
        },
    )
    .unwrap();
    rt.verify();

    let distribution = load_streams(&rt).streams.remove(1).distribution.unwrap();
    assert_eq!(
        vec![Address::new_id(RECIPIENT_B)],
        distribution.shares.iter().map(|row| row.recipient).collect::<Vec<_>>()
    );
}

#[test]
fn cancellation_strands_a_call_and_emits_drop_on_next_mutation() {
    let rt = base_runtime();
    let distribution = DistributionInit {
        writer: Address::new_id(WRITER),
        shares: vec![RecipientShare { recipient: Address::new_id(RECIPIENT_A), share: DENOM }],
    };
    let register_params = RegisterStreamParams {
        id: 3,
        weight: weight(0),
        distribution: Some(distribution.clone()),
        activation_epoch: 2,
    };
    let registration = PendingWrite {
        id: Some(3),
        op: PendingWriteOp::RegisterStream,
        payload: RawBytes::serialize(&RegisterStreamPayload {
            weight: weight(0),
            distribution: Some(distribution),
        })
        .unwrap(),
        effective_epoch: 2,
    };
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-queued", &registration, true);
    call(&rt, Method::RegisterStreamExported, &register_params).unwrap();
    rt.verify();

    let weights = SetWeightRecordsParams {
        updates: vec![WeightRecordUpdate { id: 3, weight: weight(pct(10)) }],
    };
    let stranded = PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: RawBytes::serialize(&weights).unwrap(),
        effective_epoch: 2,
    };
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-queued", &stranded, true);
    call(&rt, Method::SetWeightRecordsExported, &weights).unwrap();
    rt.verify();

    rt.epoch.replace(1);
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-cancelled", &registration, false);
    call(
        &rt,
        Method::CancelPendingExported,
        &CancelPendingParams { id: Some(3), op: PendingWriteOp::RegisterStream },
    )
    .unwrap();
    rt.verify();

    rt.epoch.replace(2);
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-dropped", &stranded, false);
    call(
        &rt,
        Method::CancelPendingExported,
        &CancelPendingParams { id: Some(999), op: PendingWriteOp::RemoveStream },
    )
    .unwrap();
    rt.verify();
    assert!(load_streams(&rt).pending_writes.is_empty());
}

#[test]
fn award_burns_the_sentinel_share_and_counts_it_as_residual() {
    let rt = base_runtime();
    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(WRITER));
    rt.expect_validate_caller_any();
    call(
        &rt,
        Method::SetSharesExported,
        &SetSharesParams {
            id: 2,
            shares: vec![
                RecipientShare { recipient: BURNT_FUNDS_ACTOR_ADDR, share: pct(25) },
                RecipientShare { recipient: Address::new_id(RECIPIENT_A), share: pct(25) },
                RecipientShare { recipient: BURNT_FUNDS_ACTOR_ADDR, share: pct(25) },
                RecipientShare { recipient: Address::new_id(RECIPIENT_B), share: pct(25) },
            ],
        },
    )
    .unwrap();
    rt.verify();

    let streams = load_streams(&rt);
    let distribution = streams.streams[1].distribution.as_ref().unwrap();
    assert_eq!(
        vec![Address::new_id(RECIPIENT_A), Address::new_id(RECIPIENT_B)],
        distribution.shares.iter().map(|row| row.recipient).collect::<Vec<_>>()
    );

    let mut state: State = rt.get_state();
    state.this_epoch_reward = TokenAmount::from_atto(500);
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_whole(1_100_000_000));
    // Half of stream 2's 20 atto portion is the sentinel's, so it burns with the 20 atto residual.
    expect_miner_reward(&rt, TokenAmount::from_atto(60), TokenAmount::zero(), ExitCode::OK);
    expect_payout(&rt, RECIPIENT_A, 5, ExitCode::OK);
    expect_payout(&rt, RECIPIENT_B, 5, ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(30), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(100), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(30), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(10), state.total_explicit_minted);
    assert_eq!(
        TokenAmount::from_atto(60),
        &state.total_minted_reward - &state.total_burn_minted - &state.total_explicit_minted
    );
    assert_state_invariants(&rt);

    // An all-sentinel map leaves the stream nothing to pay, so its whole portion burns.
    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(WRITER));
    rt.expect_validate_caller_any();
    call(
        &rt,
        Method::SetSharesExported,
        &SetSharesParams {
            id: 2,
            shares: vec![RecipientShare { recipient: BURNT_FUNDS_ACTOR_ADDR, share: DENOM }],
        },
    )
    .unwrap();
    rt.verify();
    assert!(load_streams(&rt).streams[1].distribution.as_ref().unwrap().shares.is_empty());

    expect_miner_reward(&rt, TokenAmount::from_atto(60), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(40), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(200), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(70), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(10), state.total_explicit_minted);
    assert_state_invariants(&rt);
}

#[test]
fn award_splits_multiple_wins_atto_exactly_and_preserves_the_stream_root() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.this_epoch_reward = TokenAmount::from_atto(17);
    let streams_root = state.streams_root;
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(100));

    let gas = TokenAmount::from_atto(4);
    let penalty = TokenAmount::from_atto(2);
    expect_miner_reward(&rt, TokenAmount::from_atto(7), penalty.clone(), ExitCode::OK);
    expect_payout(&rt, RECIPIENT_A, 1, ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(2), ExitCode::OK);
    award(&rt, gas, penalty, 2).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(6), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(2), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(1), state.total_explicit_minted);
    assert_eq!(
        TokenAmount::from_atto(3),
        &state.total_minted_reward - &state.total_burn_minted - &state.total_explicit_minted
    );
    assert_eq!(streams_root, state.streams_root);
    assert_eq!(TokenAmount::from_atto(90), *rt.balance.borrow());
}

#[test]
fn award_floors_each_block_instead_of_aggregating_the_epoch() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    streams.streams[0].weight = weight(pct(50));
    streams.streams[1].weight = weight(pct(50));
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(5);
    let streams_root = state.streams_root;
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(10));

    for _ in 0..2 {
        expect_miner_reward(&rt, TokenAmount::zero(), TokenAmount::zero(), ExitCode::OK);
        expect_burn(&rt, TokenAmount::from_atto(1), ExitCode::OK);
        award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
        rt.verify();
    }

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(2), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(2), state.total_burn_minted);
    assert_eq!(TokenAmount::zero(), state.total_explicit_minted);
    assert_eq!(streams_root, state.streams_root);
}

#[test]
fn award_reads_due_writes_from_queue_head_after_null_epochs() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    let applied = PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: RawBytes::serialize(&SetWeightRecordsParams {
            updates: vec![
                WeightRecordUpdate { id: 1, weight: weight(pct(50)) },
                WeightRecordUpdate { id: 2, weight: weight(pct(30)) },
            ],
        })
        .unwrap(),
        effective_epoch: 5,
    };
    let dropped = PendingWrite {
        id: Some(3),
        op: PendingWriteOp::SetDistribution,
        payload: RawBytes::serialize(&SetDistributionPayload { writer: Address::new_id(WRITER) })
            .unwrap(),
        effective_epoch: 5,
    };
    streams.pending_writes = vec![applied.clone(), dropped.clone()];
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(50);
    rt.replace_state(&state);
    rt.epoch.replace(10);
    // The block reward plus the gas reward the system actor credited before this call.
    rt.set_balance(TokenAmount::from_whole(1_100_000_000) + TokenAmount::from_atto(2));
    assert_state_invariants(&rt);

    expect_write_event(&rt, "write-applied", &applied, false);
    expect_write_event(&rt, "write-dropped", &dropped, false);
    let penalty = TokenAmount::from_atto(4);
    expect_miner_reward(&rt, TokenAmount::from_atto(7), penalty.clone(), ExitCode::OK);
    expect_payout(&rt, RECIPIENT_A, 3, ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(2), ExitCode::OK);
    award(&rt, TokenAmount::from_atto(2), penalty, 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    let streams = load_streams(&rt);
    assert!(streams.pending_writes.is_empty());
    assert_eq!(pct(50), streams.streams[0].weight.v_start);
    assert_eq!(pct(30), streams.streams[1].weight.v_start);
    assert_eq!(TokenAmount::from_atto(10), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(2), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(3), state.total_explicit_minted);
    assert_state_invariants(&rt);
}

#[test]
fn mutation_applies_due_write_from_queue_head() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    let write = PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: RawBytes::serialize(&SetWeightRecordsParams {
            updates: vec![WeightRecordUpdate { id: 1, weight: weight(pct(50)) }],
        })
        .unwrap(),
        effective_epoch: 5,
    };
    streams.pending_writes.push(write.clone());
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    rt.replace_state(&state);
    rt.epoch.replace(10);

    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-applied", &write, false);
    call(
        &rt,
        Method::CancelPendingExported,
        &CancelPendingParams { id: Some(999), op: PendingWriteOp::RemoveStream },
    )
    .unwrap();
    rt.verify();

    let streams = load_streams(&rt);
    assert!(streams.pending_writes.is_empty());
    assert_eq!(pct(50), streams.streams[0].weight.v_start);
}

#[test]
fn award_pays_only_gas_for_malformed_weights_until_repaired() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    streams.streams[0].weight = weight(0);
    streams.streams[0].weight.cap = DENOM + 1;
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(100));

    let gas = TokenAmount::from_atto(2);
    let penalty = TokenAmount::from_atto(3);
    expect_miner_reward(&rt, gas.clone(), penalty.clone(), ExitCode::OK);
    award(&rt, gas, penalty, 1).unwrap();
    rt.verify();

    let params = SetWeightRecordsParams {
        updates: vec![
            WeightRecordUpdate { id: 1, weight: weight(pct(60)) },
            WeightRecordUpdate { id: 2, weight: weight(pct(20)) },
        ],
    };
    let repair = PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: RawBytes::serialize(&params).unwrap(),
        effective_epoch: 2,
    };
    rt.set_caller(*EVM_ACTOR_CODE_ID, swa_actor());
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-queued", &repair, true);
    call(&rt, Method::SetWeightRecordsExported, &params).unwrap();
    rt.verify();

    rt.epoch.replace(2);
    expect_write_event(&rt, "write-applied", &repair, false);
    expect_miner_reward(&rt, TokenAmount::from_atto(3), TokenAmount::zero(), ExitCode::OK);
    expect_payout(&rt, RECIPIENT_A, 1, ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(1), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(5), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(1), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(1), state.total_explicit_minted);
}

#[test]
fn award_pays_only_gas_for_invalid_weight_envelope_until_repaired() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    streams.streams[0].weight = weight(pct(80));
    streams.streams[1].weight = weight(pct(30));
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(100));

    let gas = TokenAmount::from_atto(2);
    expect_miner_reward(&rt, gas.clone(), TokenAmount::zero(), ExitCode::OK);
    award(&rt, gas, TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::zero(), state.total_minted_reward);
    assert_eq!(TokenAmount::zero(), state.total_burn_minted);
    assert_eq!(TokenAmount::zero(), state.total_explicit_minted);

    let params = SetWeightRecordsParams {
        updates: vec![
            WeightRecordUpdate { id: 1, weight: weight(pct(60)) },
            WeightRecordUpdate { id: 2, weight: weight(pct(20)) },
        ],
    };
    let repair = PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: RawBytes::serialize(&params).unwrap(),
        effective_epoch: 2,
    };
    rt.set_caller(*EVM_ACTOR_CODE_ID, swa_actor());
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-queued", &repair, true);
    call(&rt, Method::SetWeightRecordsExported, &params).unwrap();
    rt.verify();

    rt.epoch.replace(2);
    expect_write_event(&rt, "write-applied", &repair, false);
    expect_miner_reward(&rt, TokenAmount::from_atto(3), TokenAmount::zero(), ExitCode::OK);
    expect_payout(&rt, RECIPIENT_A, 1, ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(1), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(5), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(1), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(1), state.total_explicit_minted);
}

#[test]
fn gate_write_for_a_removed_stream_reverts_at_admission() {
    let rt = base_runtime();
    let removal = PendingWrite {
        id: Some(2),
        op: PendingWriteOp::RemoveStream,
        payload: RawBytes::new(vec![0x80]),
        effective_epoch: 2,
    };
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-queued", &removal, true);
    call(&rt, Method::RemoveStreamExported, &RemoveStreamParams { id: 2 }).unwrap();
    rt.verify();

    rt.epoch.replace(2);
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-applied", &removal, false);
    call(
        &rt,
        Method::CancelPendingExported,
        &CancelPendingParams { id: Some(999), op: PendingWriteOp::SetDistribution },
    )
    .unwrap();
    rt.verify();
    assert_eq!(
        vec![1],
        load_streams(&rt).streams.iter().map(|stream| stream.id).collect::<Vec<_>>()
    );

    let state_before: State = rt.get_state();
    let params = SetWeightRecordsParams {
        updates: vec![WeightRecordUpdate { id: 2, weight: weight(pct(20)) }],
    };
    rt.set_caller(*EVM_ACTOR_CODE_ID, swa_actor());
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        call(&rt, Method::StepWeightRecordsExported, &params),
    );
    rt.verify();
    let state_after: State = rt.get_state();
    assert_eq!(state_before.streams_root, state_after.streams_root);
}

#[test]
fn zero_explicit_streams_are_a_stable_award_state() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    streams.streams.truncate(1);
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_whole(1_100_000_000));
    let streams_root = state.streams_root;

    for award_count in 1..=3 {
        expect_miner_reward(&rt, TokenAmount::from_atto(3), TokenAmount::zero(), ExitCode::OK);
        expect_burn(&rt, TokenAmount::from_atto(2), ExitCode::OK);
        award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
        rt.verify();

        let state: State = rt.get_state();
        assert_eq!(TokenAmount::from_atto(5 * award_count), state.total_minted_reward);
        assert_eq!(TokenAmount::from_atto(2 * award_count), state.total_burn_minted);
        assert_eq!(TokenAmount::zero(), state.total_explicit_minted);
        assert_eq!(streams_root, state.streams_root);
        assert_state_invariants(&rt);
    }
}

#[test]
fn consensus_stream_removal_uses_the_normal_queue_and_award_paths() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    streams.streams.truncate(1);
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_whole(1_100_000_000) + TokenAmount::from_atto(2));

    let removal = PendingWrite {
        id: Some(1),
        op: PendingWriteOp::RemoveStream,
        payload: RawBytes::new(vec![0x80]),
        effective_epoch: 2,
    };
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-queued", &removal, true);
    call(&rt, Method::RemoveStreamExported, &RemoveStreamParams { id: 1 }).unwrap();
    rt.verify();
    assert_state_invariants(&rt);

    rt.epoch.replace(2);
    expect_write_event(&rt, "write-applied", &removal, false);
    expect_miner_reward(&rt, TokenAmount::from_atto(2), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(5), ExitCode::OK);
    award(&rt, TokenAmount::from_atto(2), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert!(load_streams(&rt).streams.is_empty());
    assert_eq!(TokenAmount::from_atto(5), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(5), state.total_burn_minted);
    assert_eq!(TokenAmount::zero(), state.total_explicit_minted);
    assert_state_invariants(&rt);
}
#[test]
fn award_pays_only_gas_for_malformed_stream_state() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    streams.streams[1].distribution.as_mut().unwrap().shares = vec![
        RecipientShare { recipient: Address::new_id(RECIPIENT_B), share: DENOM / 2 },
        RecipientShare { recipient: Address::new_id(RECIPIENT_A), share: DENOM - DENOM / 2 },
    ];
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    let before = state.clone();
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(100));

    let gas = TokenAmount::from_atto(2);
    expect_miner_reward(&rt, gas.clone(), TokenAmount::zero(), ExitCode::OK);
    award(&rt, gas, TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(before.streams_root, state.streams_root);
    assert_eq!(before.total_minted_reward, state.total_minted_reward);
    assert_eq!(before.total_burn_minted, state.total_burn_minted);
    assert_eq!(before.total_explicit_minted, state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(98), *rt.balance.borrow());
}

#[test]
fn award_pays_only_gas_for_an_unreadable_streams_root() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.streams_root = rt.store.put_cbor(&1_u64, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    let before = state.clone();
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(100));

    let gas = TokenAmount::from_atto(2);
    expect_miner_reward(&rt, gas.clone(), TokenAmount::zero(), ExitCode::OK);
    award(&rt, gas, TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(before.streams_root, state.streams_root);
    assert_eq!(before.total_minted_reward, state.total_minted_reward);
    assert_eq!(before.total_burn_minted, state.total_burn_minted);
    assert_eq!(before.total_explicit_minted, state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(98), *rt.balance.borrow());
}

#[test]
fn award_aborts_for_a_missing_streams_root() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.streams_root = cid::Cid::new_v1(CBOR, Code::Blake2b256.digest(b"missing streams state"));
    let before = state.clone();
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(100));

    expect_abort(
        ExitCode::USR_ILLEGAL_STATE,
        award(&rt, TokenAmount::from_atto(2), TokenAmount::zero(), 1),
    );
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(before.streams_root, state.streams_root);
    assert_eq!(before.total_minted_reward, state.total_minted_reward);
    assert_eq!(before.total_burn_minted, state.total_burn_minted);
    assert_eq!(before.total_explicit_minted, state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(100), *rt.balance.borrow());
}

#[test]
fn award_pays_only_gas_when_the_balance_does_not_exceed_the_gas_reward() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    let queued = PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: RawBytes::serialize(&SetWeightRecordsParams {
            updates: vec![WeightRecordUpdate { id: 1, weight: weight(pct(50)) }],
        })
        .unwrap(),
        effective_epoch: 0,
    };
    streams.pending_writes = vec![queued.clone()];
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    let before = state.clone();
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(2));

    // The miner takes the gas reward, no recipient is paid, and nothing is burnt.
    let gas = TokenAmount::from_atto(2);
    expect_miner_reward(&rt, gas.clone(), TokenAmount::zero(), ExitCode::OK);
    award(&rt, gas, TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(before.streams_root, state.streams_root);
    assert_eq!(before.total_minted_reward, state.total_minted_reward);
    assert_eq!(before.total_burn_minted, state.total_burn_minted);
    assert_eq!(before.total_explicit_minted, state.total_explicit_minted);
    // The due write is still queued, so the next award applies it.
    assert_eq!(vec![queued], load_streams(&rt).pending_writes);
    assert_eq!(TokenAmount::zero(), *rt.balance.borrow());

    rt.set_balance(TokenAmount::from_atto(100));
    expect_write_event(&rt, "write-applied", &load_streams(&rt).pending_writes[0], false);
    expect_miner_reward(&rt, TokenAmount::from_atto(4), TokenAmount::zero(), ExitCode::OK);
    expect_payout(&rt, RECIPIENT_A, 1, ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(2), ExitCode::OK);
    award(&rt, TokenAmount::from_atto(2), TokenAmount::zero(), 1).unwrap();
    rt.verify();
}

#[test]
fn award_stands_when_the_residual_burn_cannot_be_sent() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.this_epoch_reward = TokenAmount::from_atto(500);
    let streams_root = state.streams_root;
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_whole(1_100_000_000));

    expect_miner_reward(&rt, TokenAmount::from_atto(60), TokenAmount::zero(), ExitCode::OK);
    expect_payout(&rt, RECIPIENT_A, 20, ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(20), ExitCode::USR_FORBIDDEN);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    // The miner and the recipient are paid and the counters stand, as the award committed them.
    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(100), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(20), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(20), state.total_explicit_minted);
    assert_eq!(streams_root, state.streams_root);
}

/// Eight explicit streams with full share maps, the largest award f02 admits.
fn full_stream_table() -> StreamsState {
    StreamsState {
        streams: (0..MAX_STREAMS as u64)
            .map(|slot| Stream {
                id: 2 + slot,
                weight: weight(pct(12)),
                distribution: Some(ExplicitDistribution {
                    writer: Address::new_id(WRITER),
                    shares: (0..MAX_RECIPIENTS as u64)
                        .map(|row| RecipientShare {
                            recipient: Address::new_id(
                                FIRST_BOUND_RECIPIENT + slot * MAX_RECIPIENTS as u64 + row,
                            ),
                            share: DENOM / MAX_RECIPIENTS as u64,
                        })
                        .collect(),
                }),
            })
            .collect(),
        ..Default::default()
    }
}

#[test]
fn award_pays_every_recipient_of_a_full_stream_table() {
    let rt = base_runtime();
    let streams = full_stream_table();
    let mut state: State = rt.get_state();
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    // Epoch reward 160000 attos over five expected leaders gives a 32000 atto block reward. Each
    // stream takes 12% (3840), each of its 64 recipients 60, and the unassigned 4% burns.
    state.this_epoch_reward = TokenAmount::from_atto(160_000);
    let streams_root = state.streams_root;
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_whole(1_100_000_000));

    // Expectations are consumed in order: miner, then each stream's recipients in stored order,
    // then the burn.
    expect_miner_reward(&rt, TokenAmount::zero(), TokenAmount::zero(), ExitCode::OK);
    for stream in &streams.streams {
        for row in &stream.distribution.as_ref().unwrap().shares {
            expect_payout(&rt, row.recipient.id().unwrap(), 60, ExitCode::OK);
        }
    }
    expect_burn(&rt, TokenAmount::from_atto(1_280), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(streams_root, state.streams_root);
    assert_eq!(TokenAmount::from_atto(32_000), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(30_720), state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(1_280), state.total_burn_minted);
    assert_state_invariants(&rt);
}

#[test]
fn award_sends_in_stream_order_then_recipient_order() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    // Stream 3 has the lower recipient IDs, so only stream order can put its sends after stream 2's.
    streams.streams[1].distribution.as_mut().unwrap().shares = vec![
        RecipientShare { recipient: Address::new_id(301), share: DENOM / 2 },
        RecipientShare { recipient: Address::new_id(302), share: DENOM - DENOM / 2 },
    ];
    streams.streams.push(Stream {
        id: 3,
        weight: weight(pct(15)),
        distribution: Some(ExplicitDistribution {
            writer: Address::new_id(WRITER),
            shares: vec![
                RecipientShare { recipient: Address::new_id(RECIPIENT_A), share: DENOM / 2 },
                RecipientShare {
                    recipient: Address::new_id(RECIPIENT_B),
                    share: DENOM - DENOM / 2,
                },
            ],
        }),
    });
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(500);
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_whole(1_100_000_000));

    // Stream 2 splits 20 attos evenly. Stream 3 floors 15 attos to 7 each; the odd atto burns
    // with the 5 atto residual of the 95% schedule.
    expect_miner_reward(&rt, TokenAmount::from_atto(60), TokenAmount::zero(), ExitCode::OK);
    expect_payout(&rt, 301, 10, ExitCode::OK);
    expect_payout(&rt, 302, 10, ExitCode::OK);
    expect_payout(&rt, RECIPIENT_A, 7, ExitCode::OK);
    expect_payout(&rt, RECIPIENT_B, 7, ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(6), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(100), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(34), state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(6), state.total_burn_minted);
    assert_state_invariants(&rt);
}

#[test]
fn set_shares_reads_the_writer_a_due_change_installs() {
    let rt = base_runtime();
    let next_writer = Address::new_id(NEXT_WRITER);
    rt.set_address_actor_type(next_writer, *EVM_ACTOR_CODE_ID);
    rt.set_balance(TokenAmount::from_whole(1_100_000_000));

    let change = PendingWrite {
        id: Some(2),
        op: PendingWriteOp::SetDistribution,
        payload: RawBytes::serialize(&SetDistributionPayload { writer: next_writer }).unwrap(),
        effective_epoch: 2,
    };
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-queued", &change, true);
    call(
        &rt,
        Method::SetDistributionExported,
        &SetDistributionParams { id: 2, writer: next_writer },
    )
    .unwrap();
    rt.verify();

    // The due change applies before the caller check, so the old writer is refused. The refusal
    // rolls back, leaving the change queued.
    rt.epoch.replace(2);
    let new_map = SetSharesParams {
        id: 2,
        shares: vec![RecipientShare { recipient: Address::new_id(RECIPIENT_B), share: DENOM }],
    };
    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(WRITER));
    rt.expect_validate_caller_any();
    expect_abort(ExitCode::USR_FORBIDDEN, call(&rt, Method::SetSharesExported, &new_map));
    rt.verify();
    assert_eq!(vec![change.clone()], load_streams(&rt).pending_writes);

    rt.set_caller(*EVM_ACTOR_CODE_ID, next_writer);
    rt.expect_validate_caller_any();
    expect_write_event(&rt, "write-applied", &change, false);
    call(&rt, Method::SetSharesExported, &new_map).unwrap();
    rt.verify();

    let streams = load_streams(&rt);
    let distribution = streams.streams[1].distribution.as_ref().unwrap();
    assert!(streams.pending_writes.is_empty());
    assert_eq!(next_writer, distribution.writer);
    assert_eq!(
        vec![Address::new_id(RECIPIENT_B)],
        distribution.shares.iter().map(|row| row.recipient).collect::<Vec<_>>()
    );
    assert_state_invariants(&rt);
}

#[test]
fn gas_only_award_leaves_a_due_write_for_the_next_mutation() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    let queued = PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: RawBytes::serialize(&SetWeightRecordsParams {
            updates: vec![WeightRecordUpdate { id: 2, weight: weight(pct(40)) }],
        })
        .unwrap(),
        effective_epoch: 0,
    };
    streams.pending_writes = vec![queued.clone()];
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(2));

    let gas = TokenAmount::from_atto(2);
    expect_miner_reward(&rt, gas.clone(), TokenAmount::zero(), ExitCode::OK);
    award(&rt, gas, TokenAmount::zero(), 1).unwrap();
    rt.verify();
    assert_eq!(vec![queued.clone()], load_streams(&rt).pending_writes);

    // The next mutation applies it; the award after pays under the new weight.
    rt.set_caller(*EVM_ACTOR_CODE_ID, swa_actor());
    rt.expect_validate_caller_addr(vec![swa_actor()]);
    expect_write_event(&rt, "write-applied", &queued, false);
    call(
        &rt,
        Method::CancelPendingExported,
        &CancelPendingParams { id: Some(999), op: PendingWriteOp::RemoveStream },
    )
    .unwrap();
    rt.verify();

    let streams = load_streams(&rt);
    assert!(streams.pending_writes.is_empty());
    assert_eq!(pct(40), streams.streams[1].weight.v_start);

    rt.set_balance(TokenAmount::from_atto(100));
    expect_miner_reward(&rt, TokenAmount::from_atto(3), TokenAmount::zero(), ExitCode::OK);
    expect_payout(&rt, RECIPIENT_A, 2, ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(5), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(2), state.total_explicit_minted);
    assert_eq!(TokenAmount::zero(), state.total_burn_minted);
}
