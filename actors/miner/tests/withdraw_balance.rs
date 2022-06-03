use fil_actors_runtime::test_utils::expect_abort_contains_message;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use std::ops::Deref;

mod util;
use util::*;

const BIG_BALANCE: u128 = 1_000_000_000_000_000_000_000_000u128;
const ONE_PERCENT_BALANCE: u128 = BIG_BALANCE / 100;
const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn happy_path_withdraws_funds() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    h.withdraw_funds(
        &mut rt,
        &TokenAmount::from(ONE_PERCENT_BALANCE),
        &TokenAmount::from(ONE_PERCENT_BALANCE),
        &TokenAmount::zero(),
    )
    .unwrap();
    h.check_state(&rt);
}

#[test]
fn fails_if_miner_cant_repay_fee_debt() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    let mut st = h.get_state(&rt);
    st.fee_debt = rt.balance.borrow().deref() + TokenAmount::from(1e18 as u64);
    rt.replace_state(&st);
    expect_abort_contains_message(
        ExitCode::USR_INSUFFICIENT_FUNDS,
        "unlocked balance can not repay fee debt",
        h.withdraw_funds(
            &mut rt,
            &TokenAmount::from(ONE_PERCENT_BALANCE),
            &TokenAmount::from(ONE_PERCENT_BALANCE),
            &TokenAmount::zero(),
        ),
    );
    h.check_state(&rt);
}

#[test]
fn withdraw_only_what_we_can_after_fee_debt() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    let mut st = h.get_state(&rt);
    let fee_debt = TokenAmount::from(BIG_BALANCE - ONE_PERCENT_BALANCE);
    st.fee_debt = fee_debt.clone();
    rt.replace_state(&st);

    let requested = rt.balance.borrow().to_owned();
    let expected_withdraw = &requested - &fee_debt;
    h.withdraw_funds(&mut rt, &requested, &expected_withdraw, &fee_debt).unwrap();
    h.check_state(&rt);
}
