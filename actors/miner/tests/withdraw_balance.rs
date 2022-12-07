use fil_actor_miner::BeneficiaryTerm;
use fil_actors_runtime::test_utils::{expect_abort, expect_abort_contains_message};
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;

mod util;
use util::*;

const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn happy_path_withdraws_funds() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&mut rt);

    h.withdraw_funds(
        &mut rt,
        h.owner,
        &ONE_PERCENT_BALANCE,
        &ONE_PERCENT_BALANCE,
        &TokenAmount::zero(),
    )
    .unwrap();
    h.check_state(&rt);
}

#[test]
fn fails_if_miner_cant_repay_fee_debt() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();

    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&mut rt);

    let mut st = h.get_state(&rt);
    st.fee_debt = &*rt.balance.borrow() + TokenAmount::from_whole(1);
    rt.replace_state(&st);
    expect_abort_contains_message(
        ExitCode::USR_INSUFFICIENT_FUNDS,
        "unlocked balance can not repay fee debt",
        h.withdraw_funds(
            &mut rt,
            h.owner,
            &ONE_PERCENT_BALANCE,
            &ONE_PERCENT_BALANCE,
            &TokenAmount::zero(),
        ),
    );
    rt.reset();
    h.check_state(&rt);
}

#[test]
fn withdraw_only_what_we_can_after_fee_debt() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&mut rt);

    let mut st = h.get_state(&rt);
    let fee_debt = &*BIG_BALANCE - &*ONE_PERCENT_BALANCE;
    st.fee_debt = fee_debt.clone();
    rt.replace_state(&st);

    let requested = rt.balance.borrow().to_owned();
    let expected_withdraw = &requested - &fee_debt;
    h.withdraw_funds(&mut rt, h.owner, &requested, &expected_withdraw, &fee_debt).unwrap();
    h.check_state(&rt);
}

#[test]
fn successfully_withdraw() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&mut rt);

    let one = TokenAmount::from_atto(1);
    h.withdraw_funds(&mut rt, h.owner, &one, &one, &TokenAmount::zero()).unwrap();

    let first_beneficiary_id = Address::new_id(999);
    let quota = &*ONE_PERCENT_BALANCE;
    h.propose_approve_initial_beneficiary(
        &mut rt,
        first_beneficiary_id,
        BeneficiaryTerm::new(quota.clone(), TokenAmount::zero(), PERIOD_OFFSET + 100),
    )
    .unwrap();
    h.withdraw_funds(&mut rt, h.owner, &one, &one, &TokenAmount::zero()).unwrap();
    h.withdraw_funds(&mut rt, h.beneficiary, &one, &one, &TokenAmount::zero()).unwrap();
    h.check_state(&rt);
}

#[test]
fn successfully_withdraw_allow_zero() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&mut rt);

    let first_beneficiary_id = Address::new_id(999);
    h.propose_approve_initial_beneficiary(
        &mut rt,
        first_beneficiary_id,
        BeneficiaryTerm::new(TokenAmount::from_atto(1), TokenAmount::zero(), PERIOD_OFFSET + 100),
    )
    .unwrap();
    h.withdraw_funds(
        &mut rt,
        first_beneficiary_id,
        &TokenAmount::zero(),
        &TokenAmount::zero(),
        &TokenAmount::zero(),
    )
    .unwrap();
    h.check_state(&rt);
}

#[test]
fn successfully_withdraw_limited_to_quota() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&mut rt);

    let first_beneficiary_id = Address::new_id(999);
    let quota = &*ONE_PERCENT_BALANCE;
    h.propose_approve_initial_beneficiary(
        &mut rt,
        first_beneficiary_id,
        BeneficiaryTerm::new(quota.clone(), TokenAmount::zero(), PERIOD_OFFSET + 100),
    )
    .unwrap();

    let withdraw_amount = &*ONE_PERCENT_BALANCE * 2;
    h.withdraw_funds(&mut rt, h.beneficiary, &withdraw_amount, quota, &TokenAmount::zero())
        .unwrap();
    h.check_state(&rt);
}

#[test]
fn withdraw_fail_when_beneficiary_expired() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&mut rt);

    let first_beneficiary_id = Address::new_id(999);
    let quota = &*ONE_PERCENT_BALANCE;
    h.propose_approve_initial_beneficiary(
        &mut rt,
        first_beneficiary_id,
        BeneficiaryTerm::new(quota.clone(), TokenAmount::zero(), PERIOD_OFFSET - 10),
    )
    .unwrap();
    let info = h.get_info(&rt);
    assert_eq!(PERIOD_OFFSET - 10, info.beneficiary_term.expiration);
    rt.set_epoch(100);
    let ret =
        h.withdraw_funds(&mut rt, h.beneficiary, quota, &TokenAmount::zero(), &TokenAmount::zero());
    expect_abort_contains_message(ExitCode::USR_FORBIDDEN, "beneficiary expiration of epoch", ret);
    h.check_state(&rt);
}

#[test]
fn fail_withdraw_from_non_beneficiary() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&mut rt);

    let first_beneficiary_id = Address::new_id(999);
    let another_actor = Address::new_id(1000);
    let quota = &*ONE_PERCENT_BALANCE;
    let one = TokenAmount::from_atto(1);

    expect_abort(
        ExitCode::USR_FORBIDDEN,
        h.withdraw_funds(
            &mut rt,
            first_beneficiary_id,
            &one,
            &TokenAmount::zero(),
            &TokenAmount::zero(),
        ),
    );

    h.propose_approve_initial_beneficiary(
        &mut rt,
        first_beneficiary_id,
        BeneficiaryTerm::new(quota.clone(), TokenAmount::zero(), PERIOD_OFFSET - 10),
    )
    .unwrap();

    expect_abort(
        ExitCode::USR_FORBIDDEN,
        h.withdraw_funds(&mut rt, another_actor, &one, &TokenAmount::zero(), &TokenAmount::zero()),
    );

    //allow owner withdraw
    h.withdraw_funds(&mut rt, h.owner, &one, &one, &TokenAmount::zero()).unwrap();
    //allow beneficiary withdraw
    h.withdraw_funds(&mut rt, first_beneficiary_id, &one, &one, &TokenAmount::zero()).unwrap();
    h.check_state(&rt);
}
