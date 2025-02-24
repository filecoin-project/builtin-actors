use std::ops::Neg;

use fil_actor_miner::{
    daily_fee_for_sectors, expected_reward_for_power, power_for_sectors, Actor, ApplyRewardParams,
    Method, PoStPartition,
};
use fil_actors_runtime::reward::FilterEstimate;
use fil_actors_runtime::test_utils::{MockRuntime, REWARD_ACTOR_CODE_ID};

use fil_actors_runtime::{BURNT_FUNDS_ACTOR_ADDR, REWARD_ACTOR_ADDR};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::bigint::{BigInt, Zero};
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;
use fvm_shared::{clock::ChainEpoch, econ::TokenAmount};

mod util;
use crate::util::*;

const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn fee_paid_at_deadline() {
    let (mut h, rt) = setup();
    let one_sector = h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let daily_fee = daily_fee_for_sectors(&one_sector);

    // plenty of funds available to pay fees
    let miner_balance_before = rt.get_balance();
    h.advance_and_submit_posts(&rt, &one_sector);
    let miner_balance_after = rt.get_balance();
    assert_eq!(miner_balance_before - &daily_fee, miner_balance_after);

    let mut st = h.get_state(&rt);

    // set balance to locked balance plus just enough to pay fees
    rt.set_balance(&st.initial_pledge + &daily_fee);
    h.advance_and_submit_posts(&rt, &one_sector);
    let miner_balance_after = rt.get_balance();
    assert_eq!(st.initial_pledge, miner_balance_after); // back to locked balance
    st = h.get_state(&rt);
    assert!(st.fee_debt.is_zero()); // no debt

    h.advance_and_submit_posts(&rt, &one_sector);
    assert_eq!(st.initial_pledge, miner_balance_after); // still at locked balance
    st = h.get_state(&rt);
    assert_eq!(st.fee_debt, daily_fee); // now in debt

    // set balance to pay back debt and half of the next fee
    let extra = &daily_fee.div_floor(2);
    let available_balance = &daily_fee + extra;
    rt.set_balance(&st.initial_pledge + &available_balance);
    {
        // ApplyRewards to pay back fee debt, not a normal situation; see note in ActorHarness::apply_rewards
        rt.set_caller(*REWARD_ACTOR_CODE_ID, REWARD_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![REWARD_ACTOR_ADDR]);
        rt.expect_send_simple(
            BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            None,
            daily_fee.clone(),
            None,
            ExitCode::OK,
        );
        let params = ApplyRewardParams { reward: daily_fee.clone(), penalty: TokenAmount::zero() };
        rt.call::<Actor>(Method::ApplyRewards as u64, IpldBlock::serialize_cbor(&params).unwrap())
            .unwrap();
        rt.verify();
    }

    let miner_balance_before = rt.get_balance();
    st = h.get_state(&rt);
    assert_eq!(&st.initial_pledge + extra, miner_balance_before); // back to locked balance + extra
    assert!(st.fee_debt.is_zero()); // no debt

    h.advance_and_submit_posts(&rt, &one_sector);
    let miner_balance_after = rt.get_balance();
    assert_eq!(st.initial_pledge, miner_balance_after); // back to locked balance
    st = h.get_state(&rt);
    assert_eq!(st.fee_debt, *extra); // paid back debt, but added half back

    h.check_state(&rt);
}

#[test]
fn fee_capped_by_block_reward_first() {
    test_fee_capped_by_reward(true, 1);
}

#[test]
fn fee_capped_by_block_reward_many_sectors_first() {
    test_fee_capped_by_reward(true, 55);
}

#[test]
fn fee_capped_by_block_reward_later() {
    test_fee_capped_by_reward(false, 1);
}

#[test]
fn fee_capped_by_block_reward_many_sectors_later() {
    test_fee_capped_by_reward(false, 55);
}

fn test_fee_capped_by_reward(capped_upfront: bool, num_sectors: usize) {
    // This tests two cases where half of the the estimated daily block reward for the onboarded sector
    // is less than the daily fee for the sector. In the first case, the reward is set low before sector
    // commitment, and in the second case, the reward is set low after sector commitment and before the
    // next post.

    let (mut h, rt) = setup();

    rt.set_circulating_supply(TokenAmount::from_whole(500_000_000));

    if capped_upfront {
        // set low reward before sector commitment in the capped-upfront case, this value should
        // leave us with a daily reward that's less than double the daily fee
        h.epoch_reward_smooth = FilterEstimate::new(BigInt::from(5e13 as u64), BigInt::zero());
    }

    // make sure we can pay whatever fees we need from rewards
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());

    let sectors =
        h.commit_and_prove_sectors(&rt, num_sectors, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let (dlidx, pidx) = h.get_state(&rt).find_sector(&rt.store, sectors[0].sector_number).unwrap();
    let sector_power = power_for_sectors(h.sector_size, &sectors);
    let daily_fee = daily_fee_for_sectors(&sectors);

    // in the capped-later case, expect a standard fee payment first, then set the low reward for the next payment
    if !capped_upfront {
        h.advance_and_submit_posts(&rt, &sectors);
        h.epoch_reward_smooth = FilterEstimate::new(BigInt::from(5e13 as u64), BigInt::zero());
    }

    let reward = h.epoch_reward_smooth.clone();
    let power = h.epoch_qa_power_smooth.clone();
    let day_reward = expected_reward_for_power(
        &reward,
        &power,
        &power_for_sectors(h.sector_size, &sectors).qa,
        fil_actors_runtime::EPOCHS_IN_DAY,
    );

    assert!(daily_fee < day_reward); // fee should be less than daily reward
    assert!(daily_fee > day_reward.div_floor(2)); // but greater than 50% of daily reward

    // plenty of funds available to pay fees
    let miner_balance_before = rt.get_balance();

    // manual form of h.advance_and_submit_posts to control the cron expectations

    // advance to epoch when post is due
    let mut dlinfo = h.deadline(&rt);
    while dlinfo.index != dlidx {
        dlinfo = h.advance_deadline(&rt, CronConfig::empty());
    }

    // configure post for power delta in the capped-upfront case
    let cfg = if capped_upfront {
        PoStConfig::with_expected_power_delta(&sector_power)
    } else {
        PoStConfig::empty() // no power delta, we've had first-post already
    };

    // submit post
    let partition = PoStPartition { index: pidx, skipped: make_empty_bitfield() };
    h.submit_window_post(&rt, &dlinfo, vec![partition], sectors.clone(), cfg);

    let state = h.get_state(&rt);
    let unvested = unvested_vesting_funds(&rt, &state);
    let available = rt.get_balance() + unvested.clone() - &state.initial_pledge;
    let burnt_funds = day_reward.div_floor(2); // i.e. not daily_fee
    assert!(available >= burnt_funds);
    let pledge_delta = burnt_funds.clone().neg();

    let cfg = CronConfig { burnt_funds: burnt_funds.clone(), pledge_delta, ..Default::default() };
    h.advance_deadline(&rt, cfg);

    let miner_balance_after = rt.get_balance();
    assert_eq!(miner_balance_before - burnt_funds, miner_balance_after);
}

fn setup() -> (ActorHarness, MockRuntime) {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    h.construct_and_verify(&rt);
    rt.set_balance(BIG_BALANCE.clone());

    (h, rt)
}
