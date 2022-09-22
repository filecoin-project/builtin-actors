use fil_actor_miner::BeneficiaryTerm;
use fil_actors_runtime::test_utils::{expect_abort, expect_abort_contains_message, MockRuntime};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::{address::Address, econ::TokenAmount, error::ExitCode};
use num_traits::Zero;

mod util;
use util::*;

fn setup() -> (ActorHarness, MockRuntime) {
    let big_balance = 20u128.pow(23);
    let period_offset = 100;

    let h = ActorHarness::new(period_offset);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);
    rt.balance.replace(TokenAmount::from_atto(big_balance));

    (h, rt)
}

#[test]
fn successfully_change_owner_to_another_address_two_message() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);

    let beneficiary_change = BeneficiaryChange::new(
        first_beneficiary_id,
        TokenAmount::from_atto(100),
        ChainEpoch::from(200),
    );
    // proposal beneficiary change
    h.change_beneficiary(&mut rt, h.owner, &beneficiary_change, None).unwrap();
    // assert change has been made in state
    let mut beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    let pending_beneficiary_term = beneficiary_return.proposed.unwrap();
    assert_eq!(beneficiary_change, BeneficiaryChange::from_pending(&pending_beneficiary_term));

    //confirm proposal
    h.change_beneficiary(
        &mut rt,
        first_beneficiary_id,
        &beneficiary_change,
        Some(first_beneficiary_id),
    )
    .unwrap();

    beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    assert_eq!(None, beneficiary_return.proposed);
    assert_eq!(beneficiary_change, BeneficiaryChange::from_active(&beneficiary_return.active));

    h.check_state(&rt);
}

#[test]
fn successfully_change_from_not_owner_beneficiary_to_another_address_three_message() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);
    let second_beneficiary_id = Address::new_id(1001);

    let first_beneficiary_term = BeneficiaryTerm::new(
        TokenAmount::from_atto(100),
        TokenAmount::zero(),
        ChainEpoch::from(200),
    );
    h.propose_approve_initial_beneficiary(&mut rt, first_beneficiary_id, first_beneficiary_term)
        .unwrap();

    let second_beneficiary_change = BeneficiaryChange::new(
        second_beneficiary_id,
        TokenAmount::from_atto(101),
        ChainEpoch::from(201),
    );
    h.change_beneficiary(&mut rt, h.owner, &second_beneficiary_change, None).unwrap();
    let mut beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    assert_eq!(first_beneficiary_id, beneficiary_return.active.beneficiary);

    let mut pending_beneficiary_term = beneficiary_return.proposed.unwrap();
    assert_eq!(
        second_beneficiary_change,
        BeneficiaryChange::from_pending(&pending_beneficiary_term)
    );
    assert!(!pending_beneficiary_term.approved_by_beneficiary);
    assert!(!pending_beneficiary_term.approved_by_nominee);

    h.change_beneficiary(&mut rt, second_beneficiary_id, &second_beneficiary_change, None).unwrap();
    beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    assert_eq!(first_beneficiary_id, beneficiary_return.active.beneficiary);

    pending_beneficiary_term = beneficiary_return.proposed.unwrap();
    assert_eq!(
        second_beneficiary_change,
        BeneficiaryChange::from_pending(&pending_beneficiary_term)
    );
    assert!(!pending_beneficiary_term.approved_by_beneficiary);
    assert!(pending_beneficiary_term.approved_by_nominee);

    h.change_beneficiary(&mut rt, first_beneficiary_id, &second_beneficiary_change, None).unwrap();
    beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    assert_eq!(None, beneficiary_return.proposed);
    assert_eq!(
        second_beneficiary_change,
        BeneficiaryChange::from_active(&beneficiary_return.active)
    );
    assert!(beneficiary_return.active.term.used_quota.is_zero());
}

#[test]
fn successfully_change_from_not_owner_beneficiary_to_another_address_when_beneficiary_inefficient_two_message(
) {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);
    let second_beneficiary_id = Address::new_id(1000);

    let quota = TokenAmount::from_atto(100);
    let expiration = ChainEpoch::from(200);
    h.propose_approve_initial_beneficiary(
        &mut rt,
        first_beneficiary_id,
        BeneficiaryTerm::new(quota, TokenAmount::zero(), expiration),
    )
    .unwrap();

    rt.set_epoch(201);
    let another_quota = TokenAmount::from_atto(1001);
    let another_expiration = ChainEpoch::from(3);
    let another_beneficiary_change =
        BeneficiaryChange::new(second_beneficiary_id, another_quota, another_expiration);
    h.change_beneficiary(&mut rt, h.owner, &another_beneficiary_change, None).unwrap();

    let mut beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    let pending_beneficiary_term = beneficiary_return.proposed.unwrap();
    assert_eq!(
        another_beneficiary_change,
        BeneficiaryChange::from_pending(&pending_beneficiary_term)
    );

    h.change_beneficiary(&mut rt, second_beneficiary_id, &another_beneficiary_change, None)
        .unwrap();
    beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    assert_eq!(None, beneficiary_return.proposed);
    assert_eq!(
        another_beneficiary_change,
        BeneficiaryChange::from_active(&beneficiary_return.active)
    );
    assert!(beneficiary_return.active.term.used_quota.is_zero());
    h.check_state(&rt);
}

#[test]
fn successfully_owner_immediate_revoking_unapproved_proposal() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);

    let beneficiary_change = BeneficiaryChange::new(
        first_beneficiary_id,
        TokenAmount::from_atto(100),
        ChainEpoch::from(200),
    );
    // proposal beneficiary change
    h.change_beneficiary(&mut rt, h.owner, &beneficiary_change, None).unwrap();
    // assert change has been made in state
    let mut beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    let pending_beneficiary_term = beneficiary_return.proposed.unwrap();
    assert_eq!(beneficiary_change, BeneficiaryChange::from_pending(&pending_beneficiary_term));

    //revoking unapprovel proposal
    let back_owner_beneficiary_change =
        BeneficiaryChange::new(h.owner, TokenAmount::zero(), ChainEpoch::from(0));
    h.change_beneficiary(&mut rt, h.owner, &back_owner_beneficiary_change, Some(h.owner)).unwrap();

    beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    assert_eq!(None, beneficiary_return.proposed);
    assert_eq!(
        back_owner_beneficiary_change,
        BeneficiaryChange::from_active(&beneficiary_return.active)
    );
    assert!(beneficiary_return.active.term.quota.is_zero());

    h.check_state(&rt);
}

#[test]
fn successfully_immediately_change_back_to_owner_address_while_used_up_quota() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);

    let quota = TokenAmount::from_atto(100);
    let expiration = ChainEpoch::from(200);
    h.propose_approve_initial_beneficiary(
        &mut rt,
        first_beneficiary_id,
        BeneficiaryTerm::new(quota.clone(), TokenAmount::zero(), expiration),
    )
    .unwrap();

    h.withdraw_funds(&mut rt, h.beneficiary, &quota, &quota, &TokenAmount::zero()).unwrap();
    let back_owner_beneficiary_change =
        BeneficiaryChange::new(h.owner, TokenAmount::zero(), ChainEpoch::from(0));
    h.change_beneficiary(&mut rt, h.owner, &back_owner_beneficiary_change, Some(h.owner)).unwrap();

    let beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    assert_eq!(None, beneficiary_return.proposed);
    assert_eq!(
        back_owner_beneficiary_change,
        BeneficiaryChange::from_active(&beneficiary_return.active)
    );
    assert!(beneficiary_return.active.term.quota.is_zero());
    h.check_state(&rt);
}

#[test]
fn successfully_immediately_change_back_to_owner_while_expired() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);

    let quota = TokenAmount::from_atto(100);
    let expiration = ChainEpoch::from(200);
    h.propose_approve_initial_beneficiary(
        &mut rt,
        first_beneficiary_id,
        BeneficiaryTerm::new(quota, TokenAmount::zero(), expiration),
    )
    .unwrap();

    rt.set_epoch(201);
    let back_owner_beneficiary_change =
        BeneficiaryChange::new(h.owner, TokenAmount::zero(), ChainEpoch::from(0));
    h.change_beneficiary(&mut rt, h.owner, &back_owner_beneficiary_change, Some(h.owner)).unwrap();

    let beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    assert_eq!(None, beneficiary_return.proposed);
    assert_eq!(
        back_owner_beneficiary_change,
        BeneficiaryChange::from_active(&beneficiary_return.active)
    );
    assert!(beneficiary_return.active.term.quota.is_zero());
    h.check_state(&rt);
}

#[test]
fn successfully_change_quota_and_test_withdraw() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);
    let beneficiary_term = BeneficiaryTerm::new(
        TokenAmount::from_atto(100),
        TokenAmount::zero(),
        ChainEpoch::from(200),
    );
    h.propose_approve_initial_beneficiary(&mut rt, first_beneficiary_id, beneficiary_term.clone())
        .unwrap();

    let withdraw_fund = TokenAmount::from_atto(80);
    h.withdraw_funds(&mut rt, h.beneficiary, &withdraw_fund, &withdraw_fund, &TokenAmount::zero())
        .unwrap();
    //decrease quota
    let decrease_quota = TokenAmount::from_atto(50);
    let decrease_beneficiary_change =
        BeneficiaryChange::new(first_beneficiary_id, decrease_quota, beneficiary_term.expiration);
    h.change_beneficiary(&mut rt, h.owner, &decrease_beneficiary_change, None).unwrap();
    h.change_beneficiary(
        &mut rt,
        first_beneficiary_id,
        &decrease_beneficiary_change,
        Some(first_beneficiary_id),
    )
    .unwrap();
    let mut beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    assert_eq!(
        decrease_beneficiary_change,
        BeneficiaryChange::from_active(&beneficiary_return.active)
    );
    assert_eq!(withdraw_fund, beneficiary_return.active.term.used_quota);

    //withdraw 0 zero
    let withdraw_left = TokenAmount::from_atto(20);
    let ret = h.withdraw_funds(
        &mut rt,
        h.beneficiary,
        &withdraw_left,
        &TokenAmount::zero(),
        &TokenAmount::zero(),
    );
    expect_abort_contains_message(ExitCode::USR_FORBIDDEN, "beneficiary expiration of epoch", ret);

    let increase_quota = TokenAmount::from_atto(120);
    let increase_beneficiary_change =
        BeneficiaryChange::new(first_beneficiary_id, increase_quota, beneficiary_term.expiration);

    h.change_beneficiary(&mut rt, h.owner, &increase_beneficiary_change, None).unwrap();
    h.change_beneficiary(
        &mut rt,
        first_beneficiary_id,
        &increase_beneficiary_change,
        Some(first_beneficiary_id),
    )
    .unwrap();

    beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    assert_eq!(
        increase_beneficiary_change,
        BeneficiaryChange::from_active(&beneficiary_return.active)
    );
    assert_eq!(withdraw_fund, beneficiary_return.active.term.used_quota);

    //success withdraw 40 atto fil
    let withdraw_left = TokenAmount::from_atto(40);
    h.withdraw_funds(&mut rt, h.beneficiary, &withdraw_left, &withdraw_left, &TokenAmount::zero())
        .unwrap();
    h.check_state(&rt);
}

#[test]
fn fails_approval_message_with_invalidate_params() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);

    // proposal beneficiary
    let beneficiary_change =
        &BeneficiaryChange::new(first_beneficiary_id, TokenAmount::from_atto(100), 200);
    h.change_beneficiary(&mut rt, h.owner, beneficiary_change, None).unwrap();
    let beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    assert_eq!(
        beneficiary_change,
        &BeneficiaryChange::from_pending(&beneficiary_return.proposed.unwrap())
    );

    //expiration in approval message must equal with proposal
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.change_beneficiary(
            &mut rt,
            first_beneficiary_id,
            &BeneficiaryChange::new(first_beneficiary_id, TokenAmount::from_atto(100), 201),
            None,
        ),
    );

    //quota in approval message must equal with proposal
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.change_beneficiary(
            &mut rt,
            first_beneficiary_id,
            &BeneficiaryChange::new(first_beneficiary_id, TokenAmount::from_atto(101), 200),
            None,
        ),
    );

    //beneficiary in approval message must equal with proposal
    let second_beneficiary_id = Address::new_id(1010);
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.change_beneficiary(
            &mut rt,
            first_beneficiary_id,
            &BeneficiaryChange::new(second_beneficiary_id, TokenAmount::from_atto(100), 200),
            None,
        ),
    );
}

#[test]
fn fails_proposal_beneficiary_with_invalidate_params() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);

    //not-call unable to proposal beneficiary
    expect_abort(
        ExitCode::USR_FORBIDDEN,
        h.change_beneficiary(
            &mut rt,
            first_beneficiary_id,
            &BeneficiaryChange::new(first_beneficiary_id, TokenAmount::from_atto(100), 200),
            None,
        ),
    );

    //quota must bigger than zero while change beneficiary to address(not owner)
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.change_beneficiary(
            &mut rt,
            h.owner,
            &BeneficiaryChange::new(first_beneficiary_id, TokenAmount::zero(), 200),
            None,
        ),
    );

    //quota must be zero while change beneficiary to owner address
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.change_beneficiary(
            &mut rt,
            h.owner,
            &BeneficiaryChange::new(h.owner, TokenAmount::from_atto(20), 0),
            None,
        ),
    );

    //expiration must be zero while change beneficiary to owner address
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.change_beneficiary(
            &mut rt,
            h.owner,
            &BeneficiaryChange::new(h.owner, TokenAmount::zero(), 1),
            None,
        ),
    );
}

#[test]
fn successfully_get_beneficiary() {
    let (mut h, mut rt) = setup();
    let mut beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    assert_eq!(h.owner, beneficiary_return.active.beneficiary);
    assert_eq!(BeneficiaryTerm::default(), beneficiary_return.active.term);

    let first_beneficiary_id = Address::new_id(999);
    let beneficiary_term = BeneficiaryTerm::new(
        TokenAmount::from_atto(100),
        TokenAmount::zero(),
        ChainEpoch::from(200),
    );
    h.propose_approve_initial_beneficiary(&mut rt, first_beneficiary_id, beneficiary_term).unwrap();

    let beneficiary = h.get_beneficiary(&mut rt).unwrap();
    let mut info = h.get_info(&rt);
    assert_eq!(beneficiary.active.beneficiary, info.beneficiary);
    assert_eq!(beneficiary.active.term.expiration, info.beneficiary_term.expiration);
    assert_eq!(beneficiary.active.term.quota, info.beneficiary_term.quota);
    assert_eq!(beneficiary.active.term.used_quota, info.beneficiary_term.used_quota);

    let withdraw_fund = TokenAmount::from_atto(40);
    h.withdraw_funds(&mut rt, h.beneficiary, &withdraw_fund, &withdraw_fund, &TokenAmount::zero())
        .unwrap();

    beneficiary_return = h.get_beneficiary(&mut rt).unwrap();
    info = h.get_info(&rt);
    assert_eq!(beneficiary_return.active.beneficiary, info.beneficiary);
    assert_eq!(beneficiary_return.active.term, info.beneficiary_term);
}
