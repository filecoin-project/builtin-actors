use fil_actor_miner::locked_reward_from_reward;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

mod util;
use util::*;

const BIG_BALANCE: u128 = 1_000_000_000_000_000_000_000_000u128;
const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn repay_with_no_available_funds_does_nothing() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);

    // introduce fee debt
    let mut st = h.get_state(&rt);
    let fee_debt: TokenAmount = 4 * TokenAmount::from(BIG_BALANCE);
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
    let fee_debt: TokenAmount = 4 * TokenAmount::from(BIG_BALANCE);
    st.fee_debt = fee_debt.clone();
    rt.replace_state(&st);

    let debt_to_repay = 2 * &fee_debt;
    h.repay_debts(&mut rt, &debt_to_repay, &TokenAmount::zero(), &fee_debt).unwrap();

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
    let fee_debt: TokenAmount = 4 * TokenAmount::from(BIG_BALANCE);
    st.fee_debt = fee_debt.clone();
    rt.replace_state(&st);

    let debt_to_repay = 3 * (&fee_debt / 4);
    h.repay_debts(&mut rt, &debt_to_repay, &TokenAmount::zero(), &debt_to_repay).unwrap();

    let st = h.get_state(&rt);
    assert_eq!(&fee_debt / 4, st.fee_debt);
    h.check_state(&rt);
}

#[test]
fn pay_debt_partially_from_vested_funds() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);

    let reward_amount: TokenAmount = 4 * TokenAmount::from(BIG_BALANCE);
    let (amount_locked, _) = locked_reward_from_reward(reward_amount.clone());
    rt.set_balance(amount_locked.clone());
    h.apply_rewards(&mut rt, reward_amount, TokenAmount::zero());
    assert_eq!(amount_locked, h.get_locked_funds(&rt));

    // introduce fee debt
    let mut st = h.get_state(&rt);
    st.fee_debt = 4 * TokenAmount::from(BIG_BALANCE);
    rt.replace_state(&st);

    // send 1 FIL and repay all debt from vesting funds and balance
    h.repay_debts(
        &mut rt,
        &TokenAmount::from(BIG_BALANCE), // send 1 FIL
        &amount_locked,                  // 3 FIL comes from vesting funds
        &TokenAmount::from(BIG_BALANCE), // 1 FIL sent from balance
    )
    .unwrap();

    let st = h.get_state(&rt);
    assert!(st.fee_debt.is_zero());
    h.check_state(&rt);
}
