// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::cell::RefCell;

use fil_actor_reward::{
    Actor as RewardActor, AwardBlockRewardParams, BASELINE_INITIAL_VALUE, BASELINE_TOTAL, DENOM,
    ExplicitDistribution, Method, PENALTY_MULTIPLIER, PendingWrite, PendingWriteOp,
    RecipientAmount, RecipientShare, SIMPLE_TOTAL, SetWeightRecordsParams, State, Stream,
    StreamAccrual, StreamsState, ThisEpochRewardReturn, Tombstone, WeightRecord,
    WeightRecordUpdate, ext, testing::check_state_invariants,
};
use fil_actors_runtime::EXPECTED_LEADERS_PER_EPOCH;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    ActorError, BURNT_FUNDS_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{CborStore, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntSer;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::StoragePower;
use fvm_shared::{METHOD_CONSTRUCTOR, METHOD_SEND};
use lazy_static::lazy_static;
use multihash_codetable::Code;
use num_traits::FromPrimitive;

lazy_static! {
    static ref EPOCH_ZERO_REWARD: TokenAmount =
        TokenAmount::from_atto(36_266_264_293_777_134_739i128);
    static ref WINNER: Address = Address::new_id(1000);
}

mod construction_tests {
    use num::Zero;

    use super::*;

    #[test]
    fn construct_with_zero_power() {
        let start_realized_power = StoragePower::from(0);
        let rt = construct_and_verify(&start_realized_power);

        let state: State = rt.get_state();

        assert_eq!(ChainEpoch::from(0), state.epoch);
        assert_eq!(start_realized_power, state.cumsum_realized);
        assert_eq!(*EPOCH_ZERO_REWARD, state.this_epoch_reward);
        assert_eq!(&*BASELINE_INITIAL_VALUE - 1, state.this_epoch_baseline_power);
        assert_eq!(&*BASELINE_INITIAL_VALUE, &state.effective_baseline_power);
        assert_eq!(TokenAmount::zero(), state.total_minted_reward);
        assert_eq!(TokenAmount::zero(), state.total_burn_minted);
        assert_eq!(TokenAmount::zero(), state.total_explicit_minted);
        assert!(state.accrued.is_empty());
        assert_eq!(0, state.swa_timelock_epochs);

        let streams: StreamsState =
            rt.store.get_cbor(&state.streams_root).unwrap().expect("missing streams state");
        assert_eq!(StreamsState::default(), streams);
    }

    #[test]
    fn uses_canonical_reward_totals() {
        assert_eq!(TokenAmount::from_whole(330_000_000), *SIMPLE_TOTAL);
        assert_eq!(TokenAmount::from_atto(768335872210768889362796814u128), *BASELINE_TOTAL);
    }

    #[test]
    fn checks_service_accounting_invariants() {
        let rt = construct_and_verify(&StoragePower::from(0));
        let mut state: State = rt.get_state();
        let allocation = TokenAmount::from_whole(1_100_000_000);

        let (_, acc) = check_state_invariants(&state, &*rt.store, -1, 0, &allocation);
        acc.assert_empty();

        state.swa_actor = Address::new_delegated(10, &[1; 20]).unwrap();
        let (_, acc) = check_state_invariants(&state, &*rt.store, -1, 0, &allocation);
        assert!(acc.messages().iter().any(|message| message.contains("not an ID address")));
        state.swa_actor = Address::new_id(0);

        let streams = StreamsState {
            streams: vec![Stream {
                id: 2,
                weight: WeightRecord::default(),
                distribution: Some(ExplicitDistribution {
                    writer: Address::new_id(100),
                    shares: vec![RecipientShare { recipient: Address::new_id(101), share: DENOM }],
                    payable: Vec::new(),
                    claimed_period: Vec::new(),
                }),
            }],
            ..Default::default()
        };
        state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
        state.total_minted_reward = allocation;
        let (_, acc) = check_state_invariants(&state, &*rt.store, -1, 0, &TokenAmount::zero());
        assert!(acc.messages().iter().any(|message| message.contains("missing [2]")));

        state.total_explicit_minted = TokenAmount::from_atto(10);
        state.accrued = vec![StreamAccrual { id: 2, amount: TokenAmount::from_atto(10) }];

        let (_, acc) = check_state_invariants(&state, &*rt.store, -1, 0, &TokenAmount::zero());
        assert!(
            acc.messages()
                .iter()
                .any(|message| message.contains("does not cover explicit-stream liabilities"))
        );

        let (_, acc) =
            check_state_invariants(&state, &*rt.store, -1, 0, &TokenAmount::from_atto(10));
        acc.assert_empty();

        state.accrued[0].amount = TokenAmount::from_atto(-1);
        let (_, acc) =
            check_state_invariants(&state, &*rt.store, -1, 0, &TokenAmount::from_atto(10));
        let messages = acc.messages();
        assert!(messages.iter().any(|message| message.contains("explicit-stream accrual")));
        assert!(
            messages
                .iter()
                .any(|message| message.contains("error computing explicit-stream liabilities"))
        );
    }

    #[test]
    fn rejects_malformed_stream_state_in_invariant_check() {
        let rt = construct_and_verify(&StoragePower::zero());
        let allocation = TokenAmount::from_whole(1_100_000_000);
        let distribution = ExplicitDistribution {
            writer: Address::new_id(100),
            shares: vec![RecipientShare { recipient: Address::new_id(101), share: DENOM }],
            payable: Vec::new(),
            claimed_period: Vec::new(),
        };
        let mut state: State = rt.get_state();
        state.total_minted_reward = allocation.clone();
        state.swa_timelock_epochs = 2;
        state.accrued = vec![StreamAccrual { id: 2, amount: TokenAmount::zero() }];
        // This is the valid control: the unallocated 10% burns, so weights need not sum to DENOM.
        // Each case below clones it and violates one structural invariant.
        let streams = StreamsState {
            streams: vec![
                Stream {
                    id: 1,
                    weight: WeightRecord {
                        v_start: DENOM / 100 * 60,
                        slope: 0,
                        t_start: 0,
                        floor: DENOM / 100 * 60,
                        cap: DENOM / 100 * 60,
                    },
                    distribution: None,
                },
                Stream {
                    id: 2,
                    weight: WeightRecord {
                        v_start: DENOM / 100 * 30,
                        slope: 0,
                        t_start: 0,
                        floor: DENOM / 100 * 30,
                        cap: DENOM / 100 * 30,
                    },
                    distribution: Some(distribution.clone()),
                },
            ],
            ..Default::default()
        };
        let assert_message = |state: &State, streams: &StreamsState, expected: &str| {
            let mut state = state.clone();
            state.streams_root = rt.store.put_cbor(streams, Code::Blake2b256).unwrap();
            let (_, acc) = check_state_invariants(&state, &*rt.store, -1, 0, &allocation);
            assert!(
                acc.messages().iter().any(|message| message.contains(expected)),
                "missing invariant message {expected:?} in {:?}",
                acc.messages()
            );
        };

        let mut valid_state = state.clone();
        valid_state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
        let (_, acc) = check_state_invariants(&valid_state, &*rt.store, -1, 0, &allocation);
        acc.assert_empty();

        let mut duplicate_streams = streams.clone();
        duplicate_streams.streams.insert(1, duplicate_streams.streams[0].clone());
        assert_message(&state, &duplicate_streams, "streams are not strictly ordered");

        let mut overlap = streams.clone();
        overlap.tombstones = vec![Tombstone {
            id: 2,
            payable: vec![RecipientAmount {
                recipient: Address::new_id(101),
                amount: TokenAmount::from_atto(1),
            }],
        }];
        let mut overlap_state = state.clone();
        overlap_state.total_explicit_minted = TokenAmount::from_atto(1);
        assert_message(&overlap_state, &overlap, "both live and tombstoned");

        let tombstone = Tombstone {
            id: 3,
            payable: vec![RecipientAmount {
                recipient: Address::new_id(101),
                amount: TokenAmount::from_atto(1),
            }],
        };
        let mut duplicate_tombstones = streams.clone();
        duplicate_tombstones.tombstones = vec![tombstone.clone(), tombstone];
        let mut tombstone_state = state.clone();
        tombstone_state.total_explicit_minted = TokenAmount::from_atto(2);
        assert_message(
            &tombstone_state,
            &duplicate_tombstones,
            "tombstones are not strictly ordered",
        );

        let mut duplicate_accruals = state.clone();
        duplicate_accruals.accrued.push(duplicate_accruals.accrued[0].clone());
        assert_message(&duplicate_accruals, &streams, "accrual rows are not strictly ordered");

        let mut extra_stream = streams.streams[1].clone();
        extra_stream.id = 3;
        extra_stream.weight = WeightRecord::default();
        let mut unordered_streams = streams.clone();
        unordered_streams.streams.push(extra_stream);
        let mut unordered_accruals = state.clone();
        unordered_accruals.accrued = vec![
            StreamAccrual { id: 3, amount: TokenAmount::zero() },
            StreamAccrual { id: 2, amount: TokenAmount::zero() },
        ];
        assert_message(
            &unordered_accruals,
            &unordered_streams,
            "accrual rows are not strictly ordered",
        );

        let weights = SetWeightRecordsParams {
            updates: vec![
                WeightRecordUpdate { id: 1, weight: streams.streams[0].weight.clone() },
                WeightRecordUpdate { id: 2, weight: streams.streams[1].weight.clone() },
            ],
        };
        let pending = PendingWrite {
            id: None,
            op: PendingWriteOp::SetWeightRecords,
            payload: RawBytes::serialize(&weights).unwrap(),
            effective_epoch: 2,
        };
        let mut duplicate_pending = streams.clone();
        duplicate_pending.pending_writes =
            vec![pending.clone(), PendingWrite { effective_epoch: 3, ..pending.clone() }];
        assert_message(&state, &duplicate_pending, "duplicate pending slot");

        let mut unordered_pending = streams.clone();
        unordered_pending.pending_writes = vec![
            PendingWrite { effective_epoch: 3, ..pending.clone() },
            PendingWrite { op: PendingWriteOp::StepWeightRecords, effective_epoch: 2, ..pending },
        ];
        assert_message(&state, &unordered_pending, "not ordered by effective epoch");

        let mut negative_timelock = state.clone();
        negative_timelock.swa_timelock_epochs = -1;
        assert_message(&negative_timelock, &streams, "SWA timelock is negative");

        let mut non_id_writer = streams.clone();
        non_id_writer.streams[1].distribution.as_mut().unwrap().writer =
            Address::new_delegated(10, &[1; 20]).unwrap();
        assert_message(&state, &non_id_writer, "writer");
    }

    #[test]
    fn validates_stream_schedules_from_the_runtime_epoch() {
        let rt = construct_and_verify(&StoragePower::from(0));
        let mut state: State = rt.get_state();
        let streams = StreamsState {
            streams: vec![
                Stream {
                    id: 1,
                    weight: WeightRecord {
                        v_start: DENOM,
                        slope: -((DENOM / 100 * 4) as i64),
                        t_start: 0,
                        floor: DENOM / 100 * 60,
                        cap: DENOM,
                    },
                    distribution: None,
                },
                Stream {
                    id: 2,
                    weight: WeightRecord {
                        v_start: DENOM / 100 * 20,
                        slope: 0,
                        t_start: 0,
                        floor: DENOM / 100 * 20,
                        cap: DENOM / 100 * 20,
                    },
                    distribution: Some(ExplicitDistribution {
                        writer: Address::new_id(100),
                        shares: vec![RecipientShare {
                            recipient: Address::new_id(101),
                            share: DENOM,
                        }],
                        payable: Vec::new(),
                        claimed_period: Vec::new(),
                    }),
                },
            ],
            ..Default::default()
        };
        state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
        state.accrued = vec![StreamAccrual { id: 2, amount: TokenAmount::zero() }];
        let allocation = TokenAmount::from_whole(1_100_000_000);

        let (_, at_epoch_zero) = check_state_invariants(&state, &*rt.store, -1, 0, &allocation);
        assert!(
            at_epoch_zero
                .messages()
                .iter()
                .any(|message| message.contains("invalid streams state"))
        );

        let (_, after_decline) = check_state_invariants(&state, &*rt.store, -1, 10, &allocation);
        after_decline.assert_empty();
    }

    #[test]
    fn construct_with_less_power_than_baseline() {
        let start_realized_power = StoragePower::from(1_i64 << 39);
        let rt = construct_and_verify(&start_realized_power);

        let state: State = rt.get_state();
        assert_eq!(ChainEpoch::from(0), state.epoch);
        assert_eq!(start_realized_power, state.cumsum_realized);
        assert_eq!(TokenAmount::from_atto(36_266_280_362_400_665_776i128), state.this_epoch_reward);
    }

    #[test]
    fn construct_with_more_power_than_baseline() {
        let mut start_realized_power = BASELINE_INITIAL_VALUE.clone();
        let rt = construct_and_verify(&start_realized_power);

        let state: State = rt.get_state();
        let reward = state.this_epoch_reward;

        // start with 2x power
        start_realized_power *= 2;
        let rt = construct_and_verify(&start_realized_power);

        let state: State = rt.get_state();
        assert_eq!(reward, state.this_epoch_reward);
    }
}

mod test_award_block_reward {
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::METHOD_SEND;
    use fvm_shared::error::ExitCode;
    use fvm_shared::sector::StoragePower;
    use num::Zero;

    use super::*;

    #[test]
    fn rejects_gas_reward_exceeding_balance() {
        let rt = construct_and_verify(&StoragePower::default());

        rt.set_balance(TokenAmount::from_atto(9));
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

        let params = IpldBlock::serialize_cbor(&AwardBlockRewardParams {
            miner: *WINNER,
            penalty: TokenAmount::zero(),
            gas_reward: TokenAmount::from_atto(10),
            win_count: 1,
        })
        .unwrap();
        let result = rt.call::<RewardActor>(Method::AwardBlockReward as u64, params);

        expect_abort(ExitCode::USR_ILLEGAL_STATE, result);
    }

    #[test]
    fn rejects_negative_penalty_or_reward() {
        let rt = construct_and_verify(&StoragePower::default());
        rt.set_balance(TokenAmount::from_whole(1));

        let reward_penalty_pairs = [(-1, 0), (0, -1)];

        for (reward, penalty) in &reward_penalty_pairs {
            rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

            let params = IpldBlock::serialize_cbor(&AwardBlockRewardParams {
                miner: *WINNER,
                penalty: TokenAmount::from_atto(*penalty),
                gas_reward: TokenAmount::from_atto(*reward),
                win_count: 1,
            })
            .unwrap();
            let result = rt.call::<RewardActor>(Method::AwardBlockReward as u64, params);

            expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);
            rt.reset();
        }
    }

    #[test]
    fn rejects_zero_wincount() {
        let rt = construct_and_verify(&StoragePower::default());
        rt.set_balance(TokenAmount::from_whole(1));

        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

        let params = IpldBlock::serialize_cbor(&AwardBlockRewardParams {
            miner: *WINNER,
            penalty: TokenAmount::zero(),
            gas_reward: TokenAmount::zero(),
            win_count: 0,
        })
        .unwrap();
        let result = rt.call::<RewardActor>(Method::AwardBlockReward as u64, params);

        assert!(result.is_err());
    }

    #[test]
    fn pays_reward_and_tracks_penalty() {
        let rt = construct_and_verify(&StoragePower::default());
        install_consensus_stream(&rt);
        rt.set_balance(TokenAmount::from_whole(1_000_000_000));
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        let penalty: TokenAmount = TokenAmount::from_atto(100);
        let gas_reward: TokenAmount = TokenAmount::from_atto(200);
        let expected_reward: TokenAmount =
            EPOCH_ZERO_REWARD.div_floor(EXPECTED_LEADERS_PER_EPOCH) + &gas_reward;
        let miner_penalty = PENALTY_MULTIPLIER * &penalty;
        let params = IpldBlock::serialize_cbor(&ext::miner::ApplyRewardParams {
            reward: expected_reward.clone(),
            penalty: miner_penalty,
        })
        .unwrap();
        rt.expect_send_simple(
            *WINNER,
            ext::miner::APPLY_REWARDS_METHOD,
            params,
            expected_reward,
            None,
            ExitCode::OK,
        );
        let inner_params = IpldBlock::serialize_cbor(&AwardBlockRewardParams {
            miner: *WINNER,
            penalty,
            gas_reward,
            win_count: 1,
        })
        .unwrap();
        assert!(rt.call::<RewardActor>(Method::AwardBlockReward as u64, inner_params).is_ok());
        rt.verify();
        rt.reset();
    }

    #[test]
    fn pays_out_current_balance_when_reward_exceeds_total_balance() {
        let rt = construct_and_verify(&StoragePower::from(1));
        install_consensus_stream(&rt);

        // Total reward is a huge number, upon writing ~1e18, so 300 should be way less
        let small_reward = TokenAmount::from_atto(300);
        let penalty = TokenAmount::from_atto(100);
        rt.set_balance(small_reward.clone());
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);

        let miner_penalty = PENALTY_MULTIPLIER * &penalty;
        let params = IpldBlock::serialize_cbor(&ext::miner::ApplyRewardParams {
            reward: small_reward.clone(),
            penalty: miner_penalty,
        })
        .unwrap();
        rt.expect_send_simple(
            *WINNER,
            ext::miner::APPLY_REWARDS_METHOD,
            params,
            small_reward,
            None,
            ExitCode::OK,
        );

        let inner_params = IpldBlock::serialize_cbor(&AwardBlockRewardParams {
            miner: *WINNER,
            penalty,
            gas_reward: TokenAmount::zero(),
            win_count: 1,
        })
        .unwrap();
        assert!(rt.call::<RewardActor>(Method::AwardBlockReward as u64, inner_params).is_ok());
        rt.verify();
    }

    #[test]
    fn total_mined_tracks_correctly() {
        let rt = construct_and_verify(&StoragePower::from(1));
        install_consensus_stream(&rt);
        let mut state: State = rt.get_state();

        assert_eq!(TokenAmount::zero(), state.total_minted_reward);
        state.this_epoch_reward = TokenAmount::from_atto(5000);

        rt.replace_state(&state);

        let total_payout = TokenAmount::from_atto(3500);
        rt.set_balance(total_payout.clone());

        for i in &[1000, 1000, 1000, 500] {
            assert!(
                award_block_reward(
                    &rt,
                    *WINNER,
                    TokenAmount::zero(),
                    TokenAmount::zero(),
                    1,
                    TokenAmount::from_atto(*i),
                )
                .is_ok()
            );
        }

        let new_state: State = rt.get_state();
        assert_eq!(total_payout, new_state.total_minted_reward);
    }

    #[test]
    fn funds_are_sent_to_burnt_funds_actor_if_sending_locked_funds_to_miner_fails() {
        let rt = construct_and_verify(&StoragePower::from(1));
        install_consensus_stream(&rt);
        let mut state: State = rt.get_state();

        assert_eq!(TokenAmount::zero(), state.total_minted_reward);
        state.this_epoch_reward = TokenAmount::from_atto(5000);
        rt.replace_state(&state);
        // enough balance to pay 3 full rewards and one partial
        rt.set_balance(TokenAmount::from_atto(3500));

        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        let expected_reward = TokenAmount::from_atto(1000);
        let miner_penalty = TokenAmount::zero();
        let params = IpldBlock::serialize_cbor(&ext::miner::ApplyRewardParams {
            reward: expected_reward.clone(),
            penalty: miner_penalty,
        })
        .unwrap();
        rt.expect_send_simple(
            *WINNER,
            ext::miner::APPLY_REWARDS_METHOD,
            params,
            expected_reward.clone(),
            None,
            ExitCode::USR_FORBIDDEN,
        );
        rt.expect_send_simple(
            BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            None,
            expected_reward,
            None,
            ExitCode::OK,
        );

        let inner_params = IpldBlock::serialize_cbor(&AwardBlockRewardParams {
            miner: *WINNER,
            penalty: TokenAmount::zero(),
            gas_reward: TokenAmount::zero(),
            win_count: 1,
        })
        .unwrap();

        assert!(rt.call::<RewardActor>(Method::AwardBlockReward as u64, inner_params).is_ok());

        rt.verify();
    }
}

mod test_this_epoch_reward {
    use super::*;

    #[test]
    fn successfully_fetch_reward_for_this_epoch() {
        let rt = construct_and_verify(&StoragePower::from(1));

        let state: State = rt.get_state();

        let resp: ThisEpochRewardReturn = this_epoch_reward(&rt);

        assert_eq!(state.this_epoch_baseline_power, resp.this_epoch_baseline_power);
        assert_eq!(state.this_epoch_reward_smoothed, resp.this_epoch_reward_smoothed);
    }
}

#[test]
fn test_successive_kpi_updates() {
    let power = StoragePower::from_i128(1 << 50).unwrap();
    let rt = construct_and_verify(&power);

    for i in &[1, 2, 3] {
        rt.epoch.replace(ChainEpoch::from(*i));
        update_network_kpi(&rt, &power);
    }
}

fn construct_and_verify(curr_power: &StoragePower) -> MockRuntime {
    let rt = MockRuntime {
        receiver: REWARD_ACTOR_ADDR,
        caller: RefCell::new(SYSTEM_ACTOR_ADDR),
        caller_type: RefCell::new(*SYSTEM_ACTOR_CODE_ID),
        ..Default::default()
    };
    rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    let ret = rt
        .call::<RewardActor>(
            METHOD_CONSTRUCTOR,
            IpldBlock::serialize_cbor(&(BigIntSer(curr_power))).unwrap(),
        )
        .unwrap();

    assert!(ret.is_none());
    rt.verify();
    rt
}

fn install_consensus_stream(rt: &MockRuntime) {
    let mut state: State = rt.get_state();
    let streams = StreamsState {
        streams: vec![Stream {
            id: 1,
            weight: WeightRecord { v_start: DENOM, slope: 0, t_start: 0, floor: DENOM, cap: DENOM },
            distribution: None,
        }],
        ..Default::default()
    };
    state.streams_root = rt.store.put_cbor(&streams, Code::Blake2b256).unwrap();
    rt.replace_state(&state);
}

fn award_block_reward(
    rt: &MockRuntime,
    miner: Address,
    penalty: TokenAmount,
    gas_reward: TokenAmount,
    win_count: i64,
    expected_payment: TokenAmount,
) -> Result<Option<IpldBlock>, ActorError> {
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    let miner_penalty = &penalty * PENALTY_MULTIPLIER;
    rt.expect_send_simple(
        miner,
        ext::miner::APPLY_REWARDS_METHOD,
        IpldBlock::serialize_cbor(&ext::miner::ApplyRewardParams {
            reward: expected_payment.clone(),
            penalty: miner_penalty,
        })
        .unwrap(),
        expected_payment.clone(),
        None,
        ExitCode::OK,
    );

    if penalty.is_positive() {
        rt.expect_send_simple(
            BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            None,
            expected_payment,
            None,
            ExitCode::OK,
        );
    }

    let params = IpldBlock::serialize_cbor(&AwardBlockRewardParams {
        miner,
        penalty,
        gas_reward,
        win_count,
    })
    .unwrap();

    let serialized_bytes = rt.call::<RewardActor>(Method::AwardBlockReward as u64, params)?;

    rt.verify();
    Ok(serialized_bytes)
}

fn this_epoch_reward(rt: &MockRuntime) -> ThisEpochRewardReturn {
    rt.expect_validate_caller_any();
    let serialized_result = rt.call::<RewardActor>(Method::ThisEpochReward as u64, None).unwrap();
    let resp: ThisEpochRewardReturn = serialized_result.unwrap().deserialize().unwrap();
    rt.verify();
    resp
}

fn update_network_kpi(rt: &MockRuntime, curr_raw_power: &StoragePower) {
    rt.set_caller(*POWER_ACTOR_CODE_ID, STORAGE_POWER_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![STORAGE_POWER_ACTOR_ADDR]);

    let params = IpldBlock::serialize_cbor(&(BigIntSer(curr_raw_power))).unwrap();
    assert!(rt.call::<RewardActor>(Method::UpdateNetworkKPI as u64, params).is_ok());
    rt.verify();
}
