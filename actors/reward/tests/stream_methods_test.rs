use std::cell::RefCell;

use fil_actor_reward::testing::check_state_invariants;
use fil_actor_reward::{
    Actor as RewardActor, AwardBlockRewardParams, CancelPendingParams, ClaimParams, ClaimReturn,
    DENOM, DistributionInit, ExplicitDistribution, MAX_RECIPIENTS, Method, PENALTY_MULTIPLIER,
    PendingWrite, PendingWriteOp, RecipientAmount, RecipientShare, RecipientTable,
    RegisterStreamParams, RegisterStreamPayload, RemoveStreamParams, STORAGE_MINING_ALLOCATION,
    SetDistributionParams, SetDistributionPayload, SetSharesParams, SetWeightRecordsParams, State,
    Stream, StreamAccrual, StreamsState, WeightRecord, WeightRecordUpdate, explicit_liability, ext,
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
use fvm_shared::error::ExitCode;
use multihash_codetable::{Code, MultihashDigest};
use num_traits::Zero;

const SWA_ACTOR_ID: u64 = 1001;
const WRITER: u64 = 200;
const RECIPIENT_A: u64 = 201;
const RECIPIENT_B: u64 = 202;

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
    for id in [SWA_ACTOR_ID, WRITER, RECIPIENT_A, RECIPIENT_B] {
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
                    payable: RecipientTable::default(),
                    claimed_period: RecipientTable::default(),
                }),
            },
        ],
        ..Default::default()
    };
    let streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    rt.replace_state(&State {
        epoch: 0,
        accrued: vec![StreamAccrual { id: 2, amount: TokenAmount::zero() }],
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

fn liability(rt: &MockRuntime) -> TokenAmount {
    let state: State = rt.get_state();
    explicit_liability(&load_streams(rt), &state.accrued).unwrap()
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

fn expect_claim_event(rt: &MockRuntime, stream_id: u64, recipient: Address, amount: &TokenAmount) {
    rt.expect_emitted_event(
        EventBuilder::new()
            .typ("claim-payout")
            .field_indexed("stream-id", &stream_id)
            .field_indexed("recipient", &recipient.id().unwrap())
            .field("amount", amount)
            .build()
            .unwrap(),
    );
}

fn expect_miner_reward(
    rt: &MockRuntime,
    reward: TokenAmount,
    penalty: TokenAmount,
    exit_code: ExitCode,
) {
    rt.expect_send_simple(
        Address::new_id(RECIPIENT_A),
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
        &AwardBlockRewardParams {
            miner: Address::new_id(RECIPIENT_A),
            penalty,
            gas_reward,
            win_count,
        },
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
    let too_many_wallets = vec![Address::new_id(RECIPIENT_A); MAX_RECIPIENTS + 1];

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(999));
    rt.expect_validate_caller_any();
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        call(&rt, Method::ClaimExported, &ClaimParams { id: 2, wallets: too_many_wallets }),
    );
    rt.verify();

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

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(999));
    rt.expect_validate_caller_any();
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        call(&rt, Method::ClaimExported, &ClaimParams { id: 1, wallets: Vec::new() }),
    );
    rt.verify();

    assert_eq!(before, load_streams(&rt));
}

#[test]
fn mutations_reject_inconsistent_accounting_without_a_due_write() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.accrued[0].amount = TokenAmount::from_atto(-1);
    let before = state.clone();
    rt.replace_state(&state);

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(999));
    rt.expect_validate_caller_any();
    expect_abort(
        ExitCode::USR_ILLEGAL_STATE,
        call(&rt, Method::ClaimExported, &ClaimParams { id: 999, wallets: Vec::new() }),
    );
    rt.verify();

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
    assert_eq!(before.accrued, state.accrued);
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
        Method::ClaimExported,
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
    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(300));
    rt.expect_validate_caller_any();
    expect_write_event(&rt, "write-applied", &queued, false);
    let result = call(&rt, Method::ClaimExported, &ClaimParams { id: 999, wallets: Vec::new() })
        .unwrap()
        .unwrap();
    let result: ClaimReturn = result.deserialize().unwrap();
    rt.verify();
    assert!(result.amounts.is_empty());
    let streams = load_streams(&rt);
    assert_eq!(pct(55), streams.streams[0].weight.v_start);
    assert_eq!(pct(25), streams.streams[1].weight.v_start);
    assert!(streams.pending_writes.is_empty());
}

#[test]
fn set_shares_folds_liabilities_and_burns_dust() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.accrued[0].amount = TokenAmount::from_atto(5);
    state.total_explicit_minted = TokenAmount::from_atto(5);
    let mut streams = load_streams(&rt);
    streams.streams[1].distribution.as_mut().unwrap().shares = vec![
        RecipientShare { recipient: Address::new_id(RECIPIENT_A), share: DENOM / 2 },
        RecipientShare { recipient: Address::new_id(RECIPIENT_B), share: DENOM - DENOM / 2 },
    ];
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(5));
    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(WRITER));
    rt.expect_validate_caller_any();
    rt.expect_send_simple(
        BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        None,
        TokenAmount::from_atto(1),
        None,
        ExitCode::OK,
    );

    call(
        &rt,
        Method::SetSharesExported,
        &SetSharesParams {
            id: 2,
            shares: vec![RecipientShare { recipient: Address::new_id(RECIPIENT_A), share: DENOM }],
        },
    )
    .unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::zero(), state.accrued[0].amount);
    assert_eq!(TokenAmount::from_atto(4), liability(&rt));
    let distribution = load_streams(&rt).streams.remove(1).distribution.unwrap();
    assert_eq!(
        RecipientTable::from(vec![
            RecipientAmount {
                recipient: Address::new_id(RECIPIENT_A),
                amount: TokenAmount::from_atto(2),
            },
            RecipientAmount {
                recipient: Address::new_id(RECIPIENT_B),
                amount: TokenAmount::from_atto(2),
            },
        ]),
        distribution.payable
    );
    assert_eq!(DENOM, distribution.shares[0].share);
}

#[test]
fn claim_returns_positional_amounts_and_deletes_drained_tombstone() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.accrued[0].amount = TokenAmount::from_atto(10);
    state.total_explicit_minted = TokenAmount::from_atto(17);
    let mut streams = load_streams(&rt);
    streams.tombstones.push(fil_actor_reward::Tombstone {
        id: 3,
        payable: vec![RecipientAmount {
            recipient: Address::new_id(RECIPIENT_A),
            amount: TokenAmount::from_atto(7),
        }]
        .into(),
    });
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(17));
    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(300));

    rt.expect_validate_caller_any();
    rt.expect_send_simple(
        Address::new_id(RECIPIENT_A),
        METHOD_SEND,
        None,
        TokenAmount::from_atto(10),
        None,
        ExitCode::OK,
    );
    expect_claim_event(&rt, 2, Address::new_id(RECIPIENT_A), &TokenAmount::from_atto(10));
    let result = call(
        &rt,
        Method::ClaimExported,
        &ClaimParams {
            id: 2,
            wallets: vec![
                Address::new_id(RECIPIENT_A),
                Address::new_id(RECIPIENT_A),
                Address::new_id(RECIPIENT_B),
            ],
        },
    )
    .unwrap()
    .unwrap();
    let result: ClaimReturn = result.deserialize().unwrap();
    rt.verify();
    assert_eq!(
        vec![TokenAmount::from_atto(10), TokenAmount::zero(), TokenAmount::zero()],
        result.amounts
    );
    assert_eq!(TokenAmount::from_atto(7), liability(&rt));

    rt.expect_validate_caller_any();
    let result = call(
        &rt,
        Method::ClaimExported,
        &ClaimParams { id: 2, wallets: vec![Address::new_id(RECIPIENT_A)] },
    )
    .unwrap()
    .unwrap();
    let result: ClaimReturn = result.deserialize().unwrap();
    rt.verify();
    assert_eq!(vec![TokenAmount::zero()], result.amounts);

    rt.expect_validate_caller_any();
    rt.expect_send_simple(
        Address::new_id(RECIPIENT_A),
        METHOD_SEND,
        None,
        TokenAmount::from_atto(7),
        None,
        ExitCode::OK,
    );
    expect_claim_event(&rt, 3, Address::new_id(RECIPIENT_A), &TokenAmount::from_atto(7));
    call(
        &rt,
        Method::ClaimExported,
        &ClaimParams { id: 3, wallets: vec![Address::new_id(RECIPIENT_A)] },
    )
    .unwrap();
    rt.verify();
    assert!(load_streams(&rt).tombstones.is_empty());
    assert_eq!(TokenAmount::zero(), liability(&rt));
}

#[test]
fn settlement_remains_live_while_the_weight_envelope_is_invalid() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    streams.streams[0].weight = weight(pct(90));
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.total_explicit_minted = TokenAmount::from_atto(10);
    state.accrued[0].amount = TokenAmount::from_atto(10);
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(10));

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(WRITER));
    rt.expect_validate_caller_any();
    call(
        &rt,
        Method::SetSharesExported,
        &SetSharesParams {
            id: 2,
            shares: vec![RecipientShare { recipient: Address::new_id(RECIPIENT_A), share: DENOM }],
        },
    )
    .unwrap();
    rt.verify();

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(300));
    rt.expect_validate_caller_any();
    rt.expect_send_simple(
        Address::new_id(RECIPIENT_A),
        METHOD_SEND,
        None,
        TokenAmount::from_atto(10),
        None,
        ExitCode::OK,
    );
    expect_claim_event(&rt, 2, Address::new_id(RECIPIENT_A), &TokenAmount::from_atto(10));
    let result = call(
        &rt,
        Method::ClaimExported,
        &ClaimParams { id: 2, wallets: vec![Address::new_id(RECIPIENT_A)] },
    )
    .unwrap()
    .unwrap();
    let result: ClaimReturn = result.deserialize().unwrap();
    rt.verify();

    assert_eq!(vec![TokenAmount::from_atto(10)], result.amounts);
    assert_eq!(TokenAmount::zero(), liability(&rt));
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
    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(300));
    rt.expect_validate_caller_any();
    expect_write_event(&rt, "write-dropped", &stranded, false);
    call(&rt, Method::ClaimExported, &ClaimParams { id: 999, wallets: Vec::new() }).unwrap();
    rt.verify();
    assert!(load_streams(&rt).pending_writes.is_empty());
}

#[test]
fn award_burns_sentinel_share_immediately_and_counts_it() {
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
    expect_miner_reward(&rt, TokenAmount::from_atto(60), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(30), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(100), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(30), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(10), state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(10), state.accrued[0].amount);
    assert_eq!(
        TokenAmount::from_atto(60),
        &state.total_minted_reward - &state.total_burn_minted - &state.total_explicit_minted
    );
    assert_state_invariants(&rt);

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(300));
    rt.expect_validate_caller_any();
    let result = call(
        &rt,
        Method::ClaimExported,
        &ClaimParams { id: 2, wallets: vec![BURNT_FUNDS_ACTOR_ADDR] },
    )
    .unwrap()
    .unwrap();
    let result: ClaimReturn = result.deserialize().unwrap();
    rt.verify();
    assert_eq!(vec![TokenAmount::zero()], result.amounts);
    let streams = load_streams(&rt);
    let distribution = streams.streams[1].distribution.as_ref().unwrap();
    assert!(
        distribution
            .payable
            .iter()
            .chain(distribution.claimed_period.iter())
            .all(|row| row.recipient != BURNT_FUNDS_ACTOR_ADDR)
    );

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
    let streams = load_streams(&rt);
    let distribution = streams.streams[1].distribution.as_ref().unwrap();
    assert!(distribution.shares.is_empty());
    assert!(distribution.claimed_period.is_empty());
    assert!(distribution.payable.iter().all(|row| row.recipient != BURNT_FUNDS_ACTOR_ADDR));

    expect_miner_reward(&rt, TokenAmount::from_atto(60), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(40), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(200), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(70), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(10), state.total_explicit_minted);
    assert_eq!(TokenAmount::zero(), state.accrued[0].amount);
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
    expect_burn(&rt, TokenAmount::from_atto(2), ExitCode::OK);
    award(&rt, gas, penalty, 2).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(6), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(2), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(1), state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(1), state.accrued[0].amount);
    assert_eq!(
        TokenAmount::from_atto(3),
        &state.total_minted_reward - &state.total_burn_minted - &state.total_explicit_minted
    );
    assert_eq!(streams_root, state.streams_root);
    assert_eq!(TokenAmount::from_atto(91), *rt.balance.borrow());
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
    assert_eq!(TokenAmount::zero(), state.accrued[0].amount);
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
    rt.set_balance(TokenAmount::from_whole(1_100_000_000));
    assert_state_invariants(&rt);

    expect_write_event(&rt, "write-applied", &applied, false);
    expect_write_event(&rt, "write-dropped", &dropped, false);
    let penalty = TokenAmount::from_atto(4);
    expect_miner_reward(&rt, TokenAmount::from_atto(7), penalty.clone(), ExitCode::OK);
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

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(999));
    rt.expect_validate_caller_any();
    expect_write_event(&rt, "write-applied", &write, false);
    call(&rt, Method::ClaimExported, &ClaimParams { id: 999, wallets: Vec::new() }).unwrap();
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
    expect_burn(&rt, TokenAmount::from_atto(1), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(5), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(1), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(1), state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(1), liability(&rt));
    assert_eq!(TokenAmount::from_atto(1), state.accrued[0].amount);
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
    assert_eq!(TokenAmount::zero(), state.accrued[0].amount);

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
    expect_burn(&rt, TokenAmount::from_atto(1), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(5), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(1), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(1), state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(1), state.accrued[0].amount);
}

#[test]
fn full_explicit_stream_decommission_preserves_and_drains_liabilities() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    state.total_minted_reward = TokenAmount::from_atto(10);
    state.total_explicit_minted = TokenAmount::from_atto(10);
    state.accrued[0].amount = TokenAmount::from_atto(10);
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_whole(1_100_000_000));

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
    assert_state_invariants(&rt);

    rt.epoch.replace(1);
    expect_miner_reward(&rt, TokenAmount::from_atto(3), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(1), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();
    assert_state_invariants(&rt);
    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(11), state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(11), liability(&rt));

    rt.epoch.replace(2);
    expect_write_event(&rt, "write-applied", &removal, false);
    expect_miner_reward(&rt, TokenAmount::from_atto(3), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(2), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();
    assert_state_invariants(&rt);
    let state: State = rt.get_state();
    let streams = load_streams(&rt);
    assert!(state.accrued.is_empty());
    assert_eq!(TokenAmount::from_atto(11), state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(11), liability(&rt));
    assert_eq!(1, streams.streams.len());
    assert_eq!(2, streams.tombstones[0].id);
    assert_eq!(
        TokenAmount::from_atto(11),
        streams.tombstones[0].payable.get(&Address::new_id(RECIPIENT_A))
    );

    rt.epoch.replace(3);
    expect_miner_reward(&rt, TokenAmount::from_atto(3), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(2), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();
    assert_state_invariants(&rt);
    assert_eq!(TokenAmount::from_atto(11), rt.get_state::<State>().total_explicit_minted);

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(999));
    rt.expect_validate_caller_any();
    rt.expect_send_simple(
        Address::new_id(RECIPIENT_A),
        METHOD_SEND,
        None,
        TokenAmount::from_atto(11),
        None,
        ExitCode::OK,
    );
    expect_claim_event(&rt, 2, Address::new_id(RECIPIENT_A), &TokenAmount::from_atto(11));
    let result = call(
        &rt,
        Method::ClaimExported,
        &ClaimParams { id: 2, wallets: vec![Address::new_id(RECIPIENT_A)] },
    )
    .unwrap()
    .unwrap();
    let result: ClaimReturn = result.deserialize().unwrap();
    rt.verify();
    assert_eq!(vec![TokenAmount::from_atto(11)], result.amounts);
    assert!(load_streams(&rt).tombstones.is_empty());
    assert_eq!(TokenAmount::zero(), liability(&rt));
    assert_state_invariants(&rt);

    rt.expect_validate_caller_any();
    let result = call(
        &rt,
        Method::ClaimExported,
        &ClaimParams { id: 2, wallets: vec![Address::new_id(RECIPIENT_A)] },
    )
    .unwrap()
    .unwrap();
    let result: ClaimReturn = result.deserialize().unwrap();
    rt.verify();
    assert_eq!(vec![TokenAmount::zero()], result.amounts);
    assert_state_invariants(&rt);
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
    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(999));
    rt.expect_validate_caller_any();
    expect_write_event(&rt, "write-applied", &removal, false);
    call(&rt, Method::ClaimExported, &ClaimParams { id: 999, wallets: Vec::new() }).unwrap();
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
fn zero_explicit_streams_are_a_stable_award_and_claim_state() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    streams.streams.truncate(1);
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.accrued.clear();
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
        assert_eq!(TokenAmount::zero(), liability(&rt));
        assert!(state.accrued.is_empty());
        assert_eq!(streams_root, state.streams_root);
        assert_state_invariants(&rt);
    }

    rt.set_caller(*EVM_ACTOR_CODE_ID, Address::new_id(999));
    rt.expect_validate_caller_any();
    let result = call(
        &rt,
        Method::ClaimExported,
        &ClaimParams { id: 2, wallets: vec![Address::new_id(RECIPIENT_A)] },
    )
    .unwrap()
    .unwrap();
    let result: ClaimReturn = result.deserialize().unwrap();
    rt.verify();
    assert_eq!(vec![TokenAmount::zero()], result.amounts);
    assert_state_invariants(&rt);
}

#[test]
fn consensus_stream_removal_uses_the_normal_queue_and_award_paths() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    streams.streams.truncate(1);
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.accrued.clear();
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
    assert_eq!(TokenAmount::zero(), liability(&rt));
    assert_state_invariants(&rt);
}
#[test]
fn award_burns_transition_dust_without_counting_it_as_reward_residual() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    let one_third = DENOM / 3;
    let distribution = streams.streams[1].distribution.as_mut().unwrap();
    distribution.shares = vec![
        RecipientShare { recipient: Address::new_id(RECIPIENT_A), share: one_third },
        RecipientShare { recipient: Address::new_id(RECIPIENT_B), share: DENOM - one_third },
    ];
    let removal = PendingWrite {
        id: Some(2),
        op: PendingWriteOp::RemoveStream,
        payload: RawBytes::new(vec![0x80]),
        effective_epoch: 5,
    };
    streams.pending_writes = vec![removal.clone()];
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    state.total_minted_reward = TokenAmount::from_atto(5);
    state.total_explicit_minted = TokenAmount::from_atto(5);
    state.accrued[0].amount = TokenAmount::from_atto(5);
    rt.replace_state(&state);
    rt.epoch.replace(5);
    rt.set_balance(TokenAmount::from_atto(100));

    expect_write_event(&rt, "write-applied", &removal, false);
    expect_miner_reward(&rt, TokenAmount::from_atto(3), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(3), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    let streams = load_streams(&rt);
    assert_eq!(TokenAmount::from_atto(10), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(2), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(5), state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(4), liability(&rt));
    assert!(state.accrued.is_empty());
    assert_eq!(
        TokenAmount::from_atto(4),
        streams.tombstones[0].payable.get(&Address::new_id(RECIPIENT_A))
            + streams.tombstones[0].payable.get(&Address::new_id(RECIPIENT_B))
    );
}

#[test]
fn award_reserves_existing_explicit_liabilities_when_reward_balance_is_low() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.this_epoch_reward = TokenAmount::from_atto(100);
    state.total_minted_reward = TokenAmount::from_atto(30);
    state.total_explicit_minted = TokenAmount::from_atto(30);
    state.accrued[0].amount = TokenAmount::from_atto(30);
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(40));

    expect_miner_reward(&rt, TokenAmount::from_atto(6), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(2), ExitCode::OK);
    award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(40), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(2), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(32), state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(32), state.accrued[0].amount);
    assert_eq!(TokenAmount::from_atto(32), liability(&rt));
    assert_eq!(TokenAmount::from_atto(32), *rt.balance.borrow());
}

#[test]
fn award_uses_allocation_remainder_until_invalid_explicit_accounting_is_repaired() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let supply_total = STORAGE_MINING_ALLOCATION.clone();
    let mut streams = load_streams(&rt);
    let pending = PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: RawBytes::serialize(&SetWeightRecordsParams {
            updates: vec![WeightRecordUpdate { id: 1, weight: weight(pct(60)) }],
        })
        .unwrap(),
        effective_epoch: 0,
    };
    streams.pending_writes = vec![pending.clone()];
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    state.total_minted_reward = &supply_total - TokenAmount::from_atto(3);
    state.total_burn_minted = TokenAmount::from_atto(2);
    state.total_explicit_minted = TokenAmount::from_atto(10);
    state.accrued[0].amount = TokenAmount::from_atto(-1);
    let before = state.clone();
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(15));

    let gas = TokenAmount::from_atto(2);
    let penalty = TokenAmount::from_atto(3);
    expect_miner_reward(&rt, TokenAmount::from_atto(3), penalty.clone(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(2), ExitCode::OK);
    award(&rt, gas.clone(), penalty, 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(before.streams_root, state.streams_root);
    assert_eq!(supply_total, state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(4), state.total_burn_minted);
    assert_eq!(before.total_explicit_minted, state.total_explicit_minted);
    assert_eq!(before.accrued, state.accrued);
    assert_eq!(TokenAmount::from_atto(10), *rt.balance.borrow());

    let mut repaired = state;
    repaired.total_minted_reward = &*STORAGE_MINING_ALLOCATION - TokenAmount::from_atto(5);
    repaired.accrued[0].amount = TokenAmount::from_atto(10);
    rt.replace_state(&repaired);
    rt.set_balance(TokenAmount::from_atto(100));
    expect_write_event(&rt, "write-applied", &pending, false);
    expect_miner_reward(&rt, TokenAmount::from_atto(5), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(1), ExitCode::OK);
    award(&rt, gas, TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(*STORAGE_MINING_ALLOCATION, state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(5), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(11), state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(11), state.accrued[0].amount);
}

#[test]
fn award_uses_allocation_remainder_when_a_claimed_recipient_is_absent_from_shares() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let supply_total = STORAGE_MINING_ALLOCATION.clone();
    let mut streams = load_streams(&rt);
    state.this_epoch_reward = TokenAmount::from_atto(25);
    state.total_minted_reward = &supply_total - TokenAmount::from_atto(5);
    state.total_burn_minted = TokenAmount::from_atto(2);
    state.total_explicit_minted = TokenAmount::from_atto(99);
    state.accrued[0].amount = TokenAmount::from_atto(10);
    streams.streams[1].distribution.as_mut().unwrap().claimed_period = vec![RecipientAmount {
        recipient: Address::new_id(RECIPIENT_B),
        amount: TokenAmount::from_atto(10),
    }]
    .into();
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    let before = state.clone();
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(100));

    let gas = TokenAmount::from_atto(2);
    expect_miner_reward(&rt, TokenAmount::from_atto(5), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(2), ExitCode::OK);
    award(&rt, gas, TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(before.streams_root, state.streams_root);
    assert_eq!(supply_total, state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(4), state.total_burn_minted);
    assert_eq!(before.total_explicit_minted, state.total_explicit_minted);
    assert_eq!(before.accrued, state.accrued);
    assert_eq!(TokenAmount::from_atto(93), *rt.balance.borrow());
}

#[test]
fn award_pays_only_gas_when_accounting_and_weights_are_invalid() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let supply_total = STORAGE_MINING_ALLOCATION.clone();
    let mut streams = load_streams(&rt);
    state.this_epoch_reward = TokenAmount::from_atto(25);
    state.total_minted_reward = &supply_total - TokenAmount::from_atto(5);
    state.total_burn_minted = TokenAmount::from_atto(2);
    state.total_explicit_minted = TokenAmount::from_atto(10);
    state.accrued[0].amount = TokenAmount::from_atto(-1);
    streams.streams[0].weight = weight(DENOM + 1);
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
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
    assert_eq!(before.accrued, state.accrued);
}

#[test]
fn award_pays_only_gas_when_the_allocation_remainder_is_zero() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let supply_total = STORAGE_MINING_ALLOCATION.clone();
    state.this_epoch_reward = TokenAmount::from_atto(25);
    state.total_minted_reward = supply_total;
    state.total_burn_minted = TokenAmount::from_atto(5);
    state.total_explicit_minted = TokenAmount::from_atto(30);
    state.accrued[0].amount = TokenAmount::from_atto(-1);
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
    assert_eq!(before.accrued, state.accrued);
    assert_eq!(TokenAmount::from_atto(98), *rt.balance.borrow());
}

#[test]
fn award_pays_only_gas_for_malformed_non_accounting_state() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    let mut streams = load_streams(&rt);
    streams.streams[1].distribution.as_mut().unwrap().payable = vec![RecipientAmount {
        recipient: Address::new_id(RECIPIENT_A),
        amount: TokenAmount::zero(),
    }]
    .into();
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
    assert_eq!(before.accrued, state.accrued);
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
    assert_eq!(before.accrued, state.accrued);
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
    assert_eq!(before.accrued, state.accrued);
    assert_eq!(TokenAmount::from_atto(100), *rt.balance.borrow());
}

#[test]
fn award_pays_only_gas_during_a_reserve_shortfall_then_recovers() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.this_epoch_reward = TokenAmount::from_atto(100);
    state.total_minted_reward = TokenAmount::from_atto(30);
    state.total_explicit_minted = TokenAmount::from_atto(30);
    state.accrued[0].amount = TokenAmount::from_atto(30);
    let before = state.clone();
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(31));

    let gas = TokenAmount::from_atto(2);
    expect_miner_reward(&rt, gas.clone(), TokenAmount::zero(), ExitCode::OK);
    award(&rt, gas.clone(), TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(before.streams_root, state.streams_root);
    assert_eq!(before.total_minted_reward, state.total_minted_reward);
    assert_eq!(before.total_burn_minted, state.total_burn_minted);
    assert_eq!(before.total_explicit_minted, state.total_explicit_minted);
    assert_eq!(before.accrued, state.accrued);
    assert_eq!(TokenAmount::from_atto(29), *rt.balance.borrow());

    rt.set_balance(TokenAmount::from_atto(52));
    expect_miner_reward(&rt, TokenAmount::from_atto(14), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(4), ExitCode::OK);
    award(&rt, gas, TokenAmount::zero(), 1).unwrap();
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(TokenAmount::from_atto(50), state.total_minted_reward);
    assert_eq!(TokenAmount::from_atto(4), state.total_burn_minted);
    assert_eq!(TokenAmount::from_atto(34), state.total_explicit_minted);
    assert_eq!(TokenAmount::from_atto(34), state.accrued[0].amount);
    assert_eq!(TokenAmount::from_atto(34), *rt.balance.borrow());
}

#[test]
fn award_aborts_atomically_when_the_residual_burn_fails() {
    let rt = base_runtime();
    let mut state: State = rt.get_state();
    state.this_epoch_reward = TokenAmount::from_atto(5);
    let before = state.clone();
    rt.replace_state(&state);
    rt.set_balance(TokenAmount::from_atto(10));

    expect_miner_reward(&rt, TokenAmount::zero(), TokenAmount::zero(), ExitCode::OK);
    expect_burn(&rt, TokenAmount::from_atto(1), ExitCode::USR_FORBIDDEN);
    expect_abort(ExitCode::USR_FORBIDDEN, award(&rt, TokenAmount::zero(), TokenAmount::zero(), 1));
    rt.verify();

    let state: State = rt.get_state();
    assert_eq!(before.total_minted_reward, state.total_minted_reward);
    assert_eq!(before.total_burn_minted, state.total_burn_minted);
    assert_eq!(before.total_explicit_minted, state.total_explicit_minted);
}
