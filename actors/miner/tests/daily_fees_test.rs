use fil_actor_miner::{daily_fee_for_sectors, Actor, ApplyRewardParams, Method};
use fil_actors_runtime::test_utils::{MockRuntime, REWARD_ACTOR_CODE_ID};

use fil_actors_runtime::{BURNT_FUNDS_ACTOR_ADDR, REWARD_ACTOR_ADDR};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::bigint::Zero;
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

fn setup() -> (ActorHarness, MockRuntime) {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    h.construct_and_verify(&rt);
    rt.set_balance(BIG_BALANCE.clone());

    (h, rt)
}
