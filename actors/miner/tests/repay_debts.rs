use fil_actor_miner::{locked_reward_from_reward, Actor, Method};
use fil_actors_runtime::test_utils::{expect_abort_contains_message, make_identity_cid};
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;

mod util;
use util::*;

const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn repay_with_no_available_funds_does_nothing() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);

    // introduce fee debt
    let mut st = h.get_state(&rt);
    let fee_debt: TokenAmount = 4 * &*BIG_BALANCE;
    st.fee_debt = fee_debt.clone();
    rt.replace_state(&st);

    h.repay_debts(&mut rt, &TokenAmount::zero(), &TokenAmount::zero(), &TokenAmount::zero())
        .unwrap();

    let st = h.get_state(&rt);
    assert_eq!(fee_debt, st.fee_debt);
    h.check_state(&rt);
}

#[test]
fn pay_debt_entirely_from_balance() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);

    // introduce fee debt
    let mut st = h.get_state(&rt);
    let fee_debt: TokenAmount = 4 * &*BIG_BALANCE;
    st.fee_debt = fee_debt.clone();
    rt.replace_state(&st);

    let debt_to_repay = 2 * &fee_debt;
    h.repay_debts(&mut rt, &debt_to_repay, &TokenAmount::zero(), &fee_debt).unwrap();

    let st = h.get_state(&rt);
    assert!(st.fee_debt.is_zero());
    h.check_state(&rt);
}

#[test]
fn repay_debt_restricted_correctly() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);

    // introduce fee debt
    let mut st = h.get_state(&rt);
    let fee_debt: TokenAmount = 4 * &*BIG_BALANCE;
    st.fee_debt = fee_debt.clone();
    rt.replace_state(&st);

    rt.set_caller(make_identity_cid(b"1234"), h.owner);

    // fail to call the unexported method
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "must be built-in",
        rt.call::<Actor>(Method::RepayDebt as u64, None),
    );

    // can call the exported method

    rt.expect_validate_caller_addr(h.caller_addrs());

    rt.add_balance(fee_debt.clone());
    rt.set_received(fee_debt.clone());

    rt.expect_send(BURNT_FUNDS_ACTOR_ADDR, METHOD_SEND, None, fee_debt, None, ExitCode::OK);

    rt.call::<Actor>(Method::RepayDebtExported as u64, None).unwrap();

    rt.verify();

    let st = h.get_state(&rt);
    assert!(st.fee_debt.is_zero());
    h.check_state(&rt);
}

#[test]
fn partially_repay_debt() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);

    // introduce fee debt
    let mut st = h.get_state(&rt);
    let fee_debt: TokenAmount = 4 * &*BIG_BALANCE;
    st.fee_debt = fee_debt.clone();
    rt.replace_state(&st);

    let debt_to_repay = 3 * (&fee_debt.div_floor(4));
    h.repay_debts(&mut rt, &debt_to_repay, &TokenAmount::zero(), &debt_to_repay).unwrap();

    let st = h.get_state(&rt);
    assert_eq!(fee_debt.div_floor(4), st.fee_debt);
    h.check_state(&rt);
}

#[test]
fn pay_debt_partially_from_vested_funds() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);

    let reward_amount: TokenAmount = 4 * &*BIG_BALANCE;
    let (amount_locked, _) = locked_reward_from_reward(reward_amount.clone());
    rt.set_balance(amount_locked.clone());
    h.apply_rewards(&mut rt, reward_amount, TokenAmount::zero());
    assert_eq!(amount_locked, h.get_locked_funds(&rt));

    // introduce fee debt
    let mut st = h.get_state(&rt);
    st.fee_debt = 4 * &*BIG_BALANCE;
    rt.replace_state(&st);

    // send 1 FIL and repay all debt from vesting funds and balance
    h.repay_debts(
        &mut rt,
        &BIG_BALANCE,   // send 1 FIL
        &amount_locked, // 3 FIL comes from vesting funds
        &BIG_BALANCE,   // 1 FIL sent from balance
    )
    .unwrap();

    let st = h.get_state(&rt);
    assert!(st.fee_debt.is_zero());
    h.check_state(&rt);
}
