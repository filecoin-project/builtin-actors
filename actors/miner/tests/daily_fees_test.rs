use std::collections::HashMap;
use std::ops::Neg;

use num_traits::Signed;

use fil_actor_market::{ActivatedDeal, NO_ALLOCATION_ID};
use fil_actor_miner::{
    daily_fee_for_sectors, daily_proof_fee, expected_reward_for_power,
    pledge_penalty_for_termination, power_for_sectors, qa_power_for_sector, Actor,
    ApplyRewardParams, DeadlineInfo, Method, PoStPartition, SectorOnChainInfo,
};
use fil_actors_runtime::reward::FilterEstimate;
use fil_actors_runtime::test_utils::{MockRuntime, REWARD_ACTOR_CODE_ID};
use fil_actors_runtime::{BURNT_FUNDS_ACTOR_ADDR, EPOCHS_IN_DAY, REWARD_ACTOR_ADDR};

use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::bigint::{BigInt, Zero};
use fvm_shared::error::ExitCode;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::METHOD_SEND;
use fvm_shared::{clock::ChainEpoch, econ::TokenAmount};

use test_case::test_case;

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
    assert_eq!(st.fee_debt, daily_fee - extra); // paid back debt, but added half back

    h.check_state(&rt);
}

#[test_case(true, 1; "capped upfront, single sector")]
#[test_case(true, 55; "capped upfront, many sectors")]
#[test_case(false, 1; "capped later, single sector")]
#[test_case(false, 55; "capped later, many sectors")]
fn test_fee_capped_by_reward(capped_upfront: bool, num_sectors: usize) {
    // This tests that the miner's daily fee is capped by the reward for the day. We work through
    // various sector lifecycle scenarios to ensure that the fee is correctly calculated and paid.
    //  - capped_upfront: whether the capped reward is set before sector commitment
    //  - num_sectors: number of sectors to commit

    let (mut h, rt) = setup();

    let original_epoch_reward_smooth = h.epoch_reward_smooth.clone();
    rt.set_circulating_supply(TokenAmount::from_whole(500_000_000));

    if capped_upfront {
        // set low reward before sector commitment in the capped-upfront case, this value should
        // leave us with a daily reward that's less than double the daily fee
        h.epoch_reward_smooth = FilterEstimate::new(BigInt::from(5e13 as u64), BigInt::zero());
    }

    // make sure we can pay whatever fees we need from rewards
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());

    let mut sectors =
        h.commit_and_prove_sectors(&rt, num_sectors, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let (dlidx, pidx) = h.get_state(&rt).find_sector(&rt.store, sectors[0].sector_number).unwrap();
    let sectors_power = power_for_sectors(h.sector_size, &sectors);
    let daily_fee = daily_fee_for_sectors(&sectors);

    if !capped_upfront {
        // Step 0. In the case where the fee is not capped upfront, proceed with a standard PoST and
        // expect the normal daily_fee to be applied. Change the reward after so we get the cap for
        // step 1.

        h.advance_and_submit_posts(&rt, &sectors);

        h.epoch_reward_smooth = FilterEstimate::new(BigInt::from(5e13 as u64), BigInt::zero());
    }

    let day_reward = expected_reward_for_power(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &power_for_sectors(h.sector_size, &sectors).qa,
        EPOCHS_IN_DAY,
    );

    assert!(daily_fee < day_reward); // fee should be less than daily reward
    assert!(daily_fee > day_reward.div_floor(2)); // but greater than 50% of daily reward

    // define various helper functions to keep this terse

    let advance_to_post_deadline = || -> DeadlineInfo {
        let mut dlinfo = h.deadline(&rt);
        while dlinfo.index != dlidx {
            dlinfo = h.advance_deadline(&rt, CronConfig::empty());
        }
        dlinfo
    };

    let verify_balance_change = |expected_deduction: &TokenAmount, operation: &dyn Fn()| {
        let miner_balance_before = rt.get_balance();
        operation();
        let miner_balance_after = rt.get_balance();
        assert_eq!(miner_balance_before - expected_deduction, miner_balance_after);
    };

    let submit_window_post =
        |dlinfo: &DeadlineInfo, sectors: &Vec<SectorOnChainInfo>, post_config: PoStConfig| {
            let partition = PoStPartition { index: pidx, skipped: make_empty_bitfield() };
            h.submit_window_post(&rt, dlinfo, vec![partition], sectors.clone(), post_config)
        };

    // Step 1. Normal PoST but capped by reward, not daily_fee

    let dlinfo = advance_to_post_deadline();
    // configure post for power delta in the capped-upfront case
    let cfg = if capped_upfront {
        PoStConfig::with_expected_power_delta(&sectors_power.clone())
    } else {
        PoStConfig::empty() // no power delta, we've had first-post already
    };
    submit_window_post(&dlinfo, &sectors, cfg);

    let state = h.get_state(&rt);
    let unvested = unvested_vesting_funds(&rt, &state);
    let available = rt.get_balance() + unvested.clone() - &state.initial_pledge;
    let burnt_funds = day_reward.div_floor(2); // i.e. not daily_fee
    assert!(available >= burnt_funds);

    verify_balance_change(&burnt_funds, &|| {
        h.advance_deadline(
            &rt,
            CronConfig {
                burnt_funds: burnt_funds.clone(),
                pledge_delta: burnt_funds.clone().neg(),
                ..Default::default()
            },
        );
    });

    // Step 2. Advance to next deadline, fail to submit post, make sure we have faulty power, and
    // then assert that the cap is unchanged. i.e. it includes faulty power in its calculation.

    advance_to_post_deadline();
    verify_balance_change(&burnt_funds, &|| {
        h.advance_deadline(
            &rt,
            CronConfig {
                burnt_funds: burnt_funds.clone(),
                pledge_delta: burnt_funds.clone().neg(),
                power_delta: Some(sectors_power.clone().neg()),
                ..Default::default()
            },
        );
    });

    // Step 3. Advance to next deadline and submit post, recovering power, and pay the same capped
    // fee.

    let bf = bitfield_from_slice(&sectors.iter().map(|s| s.sector_number).collect::<Vec<u64>>());
    h.declare_recoveries(&rt, dlidx, pidx, bf, TokenAmount::zero()).unwrap();
    let dlinfo = advance_to_post_deadline();
    submit_window_post(&dlinfo, &sectors, PoStConfig::with_expected_power_delta(&sectors_power));
    verify_balance_change(&burnt_funds, &|| {
        h.advance_deadline(
            &rt,
            CronConfig {
                burnt_funds: burnt_funds.clone(),
                pledge_delta: -burnt_funds.clone(),
                ..Default::default()
            },
        );
    });

    if num_sectors > 1 {
        // Step 4 (multiple sectors). Terminate a sector, make sure we pay a capped fee that is
        // proportional to the power of the remaining sectors.

        let terminated_sector = &sectors[0];
        let sector_power = qa_power_for_sector(h.sector_size, terminated_sector);
        let sector_age = *rt.epoch.borrow() - terminated_sector.power_base_epoch;
        let expected_fee = pledge_penalty_for_termination(
            &terminated_sector.expected_day_reward,
            sector_age,
            &terminated_sector.expected_storage_pledge,
            &h.epoch_qa_power_smooth,
            &sector_power,
            &h.epoch_reward_smooth,
            &TokenAmount::zero(),
            0,
        );
        h.terminate_sectors(
            &rt,
            &bitfield_from_slice(&[terminated_sector.sector_number]),
            expected_fee,
        );

        sectors.remove(0);

        let mut dlinfo = h.deadline(&rt);
        while dlinfo.index != dlidx {
            dlinfo = h.advance_deadline(&rt, CronConfig::empty());
        }

        submit_window_post(&dlinfo, &sectors, PoStConfig::empty());

        let day_reward = expected_reward_for_power(
            &h.epoch_reward_smooth,
            &h.epoch_qa_power_smooth,
            &power_for_sectors(h.sector_size, &sectors).qa,
            EPOCHS_IN_DAY,
        );
        let burnt_funds = day_reward.div_floor(2);
        verify_balance_change(&burnt_funds, &|| {
            h.advance_deadline(
                &rt,
                CronConfig {
                    burnt_funds: burnt_funds.clone(),
                    pledge_delta: burnt_funds.clone().neg(),
                    ..Default::default()
                },
            );
        });
    }

    // Step 5. Reset the reward to the original value, advance to the next deadline, and submit a
    // post. We should pay the standard daily_fee with no cap.

    h.epoch_reward_smooth = original_epoch_reward_smooth;
    let daily_fee = daily_fee_for_sectors(&sectors);
    verify_balance_change(&daily_fee, &|| {
        h.advance_and_submit_posts(&rt, &sectors);
    });
}

#[test]
fn fees_proportional_to_qap() {
    let (mut h, rt) = setup();

    let sectors = h.commit_and_prove_sectors_with_cfgs(
        &rt,
        5,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![], vec![1], vec![2], vec![3], vec![4]],
        true,
        ProveCommitConfig {
            verify_deals_exit: Default::default(),
            claim_allocs_exit: Default::default(),
            activated_deals: HashMap::from_iter(vec![
                (
                    1,
                    vec![ActivatedDeal {
                        client: 0,
                        allocation_id: NO_ALLOCATION_ID,
                        data: Default::default(),
                        size: PaddedPieceSize(h.sector_size as u64),
                    }],
                ),
                (
                    2,
                    vec![ActivatedDeal {
                        client: 0,
                        allocation_id: NO_ALLOCATION_ID,
                        data: Default::default(),
                        size: PaddedPieceSize(h.sector_size as u64 / 2),
                    }],
                ),
                (
                    3,
                    vec![ActivatedDeal {
                        client: 0,
                        allocation_id: 1,
                        data: Default::default(),
                        size: PaddedPieceSize(h.sector_size as u64),
                    }],
                ),
                (
                    4,
                    vec![ActivatedDeal {
                        client: 0,
                        allocation_id: 2,
                        data: Default::default(),
                        size: PaddedPieceSize(h.sector_size as u64 / 2),
                    }],
                ),
            ]),
        },
    );

    // for a reference we'll calculate the fee for a fully verified sector and
    // divide as required
    let full_verified_fee = daily_proof_fee(
        &rt.policy,
        &rt.circulating_supply.borrow(),
        &BigInt::from(h.sector_size as u64 * 10),
    );

    // no deals
    assert_eq!(full_verified_fee.div_floor(10), sectors[0].daily_fee);
    // deal, unverified
    assert_eq!(full_verified_fee.div_floor(10), sectors[1].daily_fee);
    // deal, half, unverified
    assert_eq!(full_verified_fee.div_floor(10), sectors[2].daily_fee);
    // deal, verified
    assert_eq!(full_verified_fee.clone(), sectors[3].daily_fee);
    // deal, half, verified
    assert!(
        ((full_verified_fee.clone() * 11).div_floor(20) - &sectors[4].daily_fee).atto().abs()
            <= BigInt::from(1)
    );
}

fn setup() -> (ActorHarness, MockRuntime) {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    h.construct_and_verify(&rt);
    rt.set_balance(BIG_BALANCE.clone());

    (h, rt)
}
