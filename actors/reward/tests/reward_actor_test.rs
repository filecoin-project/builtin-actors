// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actor_reward::{
    ext, Actor as RewardActor, AwardBlockRewardParams, Method, State, ThisEpochRewardReturn,
    BASELINE_INITIAL_VALUE, PENALTY_MULTIPLIER,
};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::EXPECTED_LEADERS_PER_EPOCH;
use fil_actors_runtime::{
    ActorError, BURNT_FUNDS_ACTOR_ADDR, REWARD_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntSer;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::StoragePower;
use fvm_shared::{METHOD_CONSTRUCTOR, METHOD_SEND};
use lazy_static::lazy_static;
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
    }

    #[test]
    fn construct_with_less_power_than_baseline() {
        let start_realized_power = StoragePower::from(1_i64 << 39);
        let rt = construct_and_verify(&start_realized_power);

        let state: State = rt.get_state();
        assert_eq!(ChainEpoch::from(0), state.epoch);
        assert_eq!(start_realized_power, state.cumsum_realized);
        assert_ne!(TokenAmount::zero(), state.this_epoch_reward);
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
    use fvm_shared::error::ExitCode;
    use fvm_shared::sector::StoragePower;
    use fvm_shared::METHOD_SEND;
    use num::Zero;

    use super::*;

    #[test]
    fn rejects_gas_reward_exceeding_balance() {
        let mut rt = construct_and_verify(&StoragePower::default());

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
        let mut rt = construct_and_verify(&StoragePower::default());
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
        let mut rt = construct_and_verify(&StoragePower::default());
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
        let mut rt = construct_and_verify(&StoragePower::default());
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
        rt.expect_send(
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
        let mut rt = construct_and_verify(&StoragePower::from(1));

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
        rt.expect_send(
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
        let mut rt = construct_and_verify(&StoragePower::from(1));
        let mut state: State = rt.get_state();

        assert_eq!(TokenAmount::zero(), state.total_storage_power_reward);
        state.this_epoch_reward = TokenAmount::from_atto(5000);

        rt.replace_state(&state);

        let total_payout = TokenAmount::from_atto(3500);
        rt.set_balance(total_payout.clone());

        for i in &[1000, 1000, 1000, 500] {
            assert!(award_block_reward(
                &mut rt,
                *WINNER,
                TokenAmount::zero(),
                TokenAmount::zero(),
                1,
                TokenAmount::from_atto(*i),
            )
            .is_ok());
        }

        let new_state: State = rt.get_state();
        assert_eq!(total_payout, new_state.total_storage_power_reward);
    }

    #[test]
    fn funds_are_sent_to_burnt_funds_actor_if_sending_locked_funds_to_miner_fails() {
        let mut rt = construct_and_verify(&StoragePower::from(1));
        let mut state: State = rt.get_state();

        assert_eq!(TokenAmount::zero(), state.total_storage_power_reward);
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
        rt.expect_send(
            *WINNER,
            ext::miner::APPLY_REWARDS_METHOD,
            params,
            expected_reward.clone(),
            None,
            ExitCode::USR_FORBIDDEN,
        );
        rt.expect_send(
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
        let mut rt = construct_and_verify(&StoragePower::from(1));

        let state: State = rt.get_state();

        let resp: ThisEpochRewardReturn = this_epoch_reward(&mut rt);

        assert_eq!(state.this_epoch_baseline_power, resp.this_epoch_baseline_power);
        assert_eq!(state.this_epoch_reward_smoothed, resp.this_epoch_reward_smoothed);
    }
}

#[test]
fn test_successive_kpi_updates() {
    let power = StoragePower::from_i128(1 << 50).unwrap();
    let mut rt = construct_and_verify(&power);

    for i in &[1, 2, 3] {
        rt.epoch = ChainEpoch::from(*i);
        update_network_kpi(&mut rt, &power);
    }
}

fn construct_and_verify(curr_power: &StoragePower) -> MockRuntime {
    let mut rt = MockRuntime {
        receiver: REWARD_ACTOR_ADDR,
        caller: SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    };
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

fn award_block_reward(
    rt: &mut MockRuntime,
    miner: Address,
    penalty: TokenAmount,
    gas_reward: TokenAmount,
    win_count: i64,
    expected_payment: TokenAmount,
) -> Result<Option<IpldBlock>, ActorError> {
    rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
    let miner_penalty = &penalty * PENALTY_MULTIPLIER;
    rt.expect_send(
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
        rt.expect_send(
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

fn this_epoch_reward(rt: &mut MockRuntime) -> ThisEpochRewardReturn {
    rt.expect_validate_caller_any();
    let serialized_result = rt.call::<RewardActor>(Method::ThisEpochReward as u64, None).unwrap();
    let resp: ThisEpochRewardReturn = serialized_result.unwrap().deserialize().unwrap();
    rt.verify();
    resp
}

fn update_network_kpi(rt: &mut MockRuntime, curr_raw_power: &StoragePower) {
    rt.set_caller(*POWER_ACTOR_CODE_ID, STORAGE_POWER_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![STORAGE_POWER_ACTOR_ADDR]);

    let params = IpldBlock::serialize_cbor(&(BigIntSer(curr_raw_power))).unwrap();
    assert!(rt.call::<RewardActor>(Method::UpdateNetworkKPI as u64, params).is_ok());
    rt.verify();
}
