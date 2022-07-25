use fil_actor_miner::BeneficiaryTerm;
use fil_actors_runtime::test_utils::{expect_abort, MockRuntime};
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
    rt.balance.replace(TokenAmount::from(big_balance));

    (h, rt)
}

#[test]
fn successfully_change_owner_to_another_address_two_message() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);

    let beneficiary_change =
        BeneficiaryChange::new(first_beneficiary_id, TokenAmount::from(100), ChainEpoch::from(200));
    // proposal beneficiary change
    h.change_beneficiary(&mut rt, h.owner, beneficiary_change.clone(), None).unwrap();
    // assert change has been made in state
    let mut info = h.get_info(&mut rt);
    let pending_beneficiary_term = info.pending_beneficiary_term.unwrap();
    assert_eq!(beneficiary_change.expiration, pending_beneficiary_term.new_expiration);
    assert_eq!(beneficiary_change.quota.clone(), pending_beneficiary_term.new_quota);
    assert_eq!(first_beneficiary_id, pending_beneficiary_term.new_beneficiary);

    //confirm proposal
    h.change_beneficiary(
        &mut rt,
        first_beneficiary_id,
        beneficiary_change.clone(),
        Some(first_beneficiary_id),
    )
    .unwrap();

    info = h.get_info(&mut rt);
    assert_eq!(None, info.pending_beneficiary_term);
    assert_eq!(first_beneficiary_id, info.beneficiary);
    assert_eq!(beneficiary_change.quota.clone(), info.beneficiary_term.quota);
    assert_eq!(beneficiary_change.expiration, info.beneficiary_term.expiration);

    h.check_state(&rt);
}

#[test]
fn successfully_change_from_not_owner_beneficiary_to_another_address_three_message() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);
    let second_beneficiary_id = Address::new_id(1001);

    let first_beneficiary_term =
        BeneficiaryTerm::new(TokenAmount::from(100), TokenAmount::zero(), ChainEpoch::from(200));
    h.propose_approve_initial_beneficiary(&mut rt, first_beneficiary_id, first_beneficiary_term)
        .unwrap();

    let second_beneficiary_change = BeneficiaryChange::new(
        second_beneficiary_id,
        TokenAmount::from(101),
        ChainEpoch::from(201),
    );
    h.change_beneficiary(&mut rt, h.owner, second_beneficiary_change.clone(), None).unwrap();
    let mut info = h.get_info(&mut rt);
    assert_eq!(first_beneficiary_id, info.beneficiary);

    let mut pending_beneficiary_term = info.pending_beneficiary_term.unwrap();
    assert_eq!(second_beneficiary_change.expiration, pending_beneficiary_term.new_expiration);
    assert_eq!(second_beneficiary_change.quota.clone(), pending_beneficiary_term.new_quota);
    assert_eq!(second_beneficiary_id, pending_beneficiary_term.new_beneficiary);
    assert!(!pending_beneficiary_term.approved_by_beneficiary);
    assert!(!pending_beneficiary_term.approved_by_nominee);

    h.change_beneficiary(&mut rt, first_beneficiary_id, second_beneficiary_change.clone(), None)
        .unwrap();
    info = h.get_info(&mut rt);
    assert_eq!(first_beneficiary_id, info.beneficiary);

    pending_beneficiary_term = info.pending_beneficiary_term.unwrap();
    assert_eq!(second_beneficiary_change.expiration, pending_beneficiary_term.new_expiration);
    assert_eq!(second_beneficiary_change.quota.clone(), pending_beneficiary_term.new_quota);
    assert_eq!(second_beneficiary_id, pending_beneficiary_term.new_beneficiary);
    assert!(pending_beneficiary_term.approved_by_beneficiary);
    assert!(!pending_beneficiary_term.approved_by_nominee);

    h.change_beneficiary(&mut rt, second_beneficiary_id, second_beneficiary_change.clone(), None)
        .unwrap();
    info = h.get_info(&mut rt);
    assert_eq!(second_beneficiary_id, info.beneficiary);

    assert_eq!(None, info.pending_beneficiary_term);
    assert_eq!(second_beneficiary_change.expiration, info.beneficiary_term.expiration);
    assert_eq!(second_beneficiary_change.quota, info.beneficiary_term.quota);
    assert_eq!(TokenAmount::zero(), info.beneficiary_term.used_quota);
}

#[test]
fn successfully_change_from_not_owner_beneficiary_to_another_address_when_beneficiary_inefficient_two_message(
) {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);
    let second_beneficiary_id = Address::new_id(1000);

    let quota = TokenAmount::from(100);
    let expiration = ChainEpoch::from(200);
    h.propose_approve_initial_beneficiary(
        &mut rt,
        first_beneficiary_id,
        BeneficiaryTerm::new(quota.clone(), TokenAmount::zero(), expiration),
    )
    .unwrap();

    rt.set_epoch(201);
    let another_quota = TokenAmount::from(1001);
    let another_expiration = ChainEpoch::from(3);
    let another_beneficiary_change =
        BeneficiaryChange::new(second_beneficiary_id, another_quota.clone(), another_expiration);
    h.change_beneficiary(&mut rt, h.owner, another_beneficiary_change.clone(), None).unwrap();
    let mut info = h.get_info(&mut rt);

    let pending_beneficiary_term = info.pending_beneficiary_term.unwrap();
    assert_eq!(second_beneficiary_id, pending_beneficiary_term.new_beneficiary);
    assert_eq!(another_quota, pending_beneficiary_term.new_quota);
    assert_eq!(another_expiration, pending_beneficiary_term.new_expiration);

    h.change_beneficiary(&mut rt, second_beneficiary_id, another_beneficiary_change, None).unwrap();
    info = h.get_info(&mut rt);
    assert_eq!(None, info.pending_beneficiary_term);
    assert_eq!(second_beneficiary_id, info.beneficiary);
    assert_eq!(another_quota, info.beneficiary_term.quota);
    assert_eq!(another_expiration, info.beneficiary_term.expiration);
    assert_eq!(TokenAmount::zero(), info.beneficiary_term.used_quota);
    h.check_state(&rt);
}

#[test]
fn successfully_owner_immediate_revoking_unapproved_proposal() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);

    let beneficiary_change =
        BeneficiaryChange::new(first_beneficiary_id, TokenAmount::from(100), ChainEpoch::from(200));
    // proposal beneficiary change
    h.change_beneficiary(&mut rt, h.owner, beneficiary_change.clone(), None).unwrap();
    // assert change has been made in state
    let mut info = h.get_info(&mut rt);
    let pending_beneficiary_term = info.pending_beneficiary_term.unwrap();
    assert_eq!(beneficiary_change.expiration, pending_beneficiary_term.new_expiration);
    assert_eq!(beneficiary_change.quota, pending_beneficiary_term.new_quota);
    assert_eq!(first_beneficiary_id, pending_beneficiary_term.new_beneficiary);

    //revoking unapprovel proposal
    h.change_beneficiary(
        &mut rt,
        h.owner,
        BeneficiaryChange::new(h.owner, TokenAmount::zero(), ChainEpoch::from(0)),
        Some(h.owner),
    )
    .unwrap();

    info = h.get_info(&mut rt);
    assert_eq!(None, info.pending_beneficiary_term);
    assert_eq!(h.owner, info.beneficiary);

    h.check_state(&rt);
}

#[test]
fn successfully_immediately_change_back_to_owner_address_while_used_up_quota() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);

    let quota = TokenAmount::from(100);
    let expiration = ChainEpoch::from(200);
    h.propose_approve_initial_beneficiary(
        &mut rt,
        first_beneficiary_id,
        BeneficiaryTerm::new(quota.clone(), TokenAmount::zero(), expiration),
    )
    .unwrap();

    h.withdraw_funds(&mut rt, h.beneficiary, &quota, &quota, &TokenAmount::zero()).unwrap();
    h.change_beneficiary(
        &mut rt,
        h.owner,
        BeneficiaryChange::new(h.owner, TokenAmount::zero(), ChainEpoch::from(0)),
        Some(h.owner),
    )
    .unwrap();

    let info = h.get_info(&mut rt);
    assert_eq!(None, info.pending_beneficiary_term);
    assert_eq!(h.owner, info.beneficiary);
    assert_eq!(TokenAmount::zero(), info.beneficiary_term.quota);
    assert_eq!(ChainEpoch::from(0), info.beneficiary_term.expiration);
    assert_eq!(TokenAmount::zero(), info.beneficiary_term.used_quota);
    h.check_state(&rt);
}

#[test]
fn successfully_immediately_change_back_to_owner_while_expired() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);

    let quota = TokenAmount::from(100);
    let expiration = ChainEpoch::from(200);
    h.propose_approve_initial_beneficiary(
        &mut rt,
        first_beneficiary_id,
        BeneficiaryTerm::new(quota.clone(), TokenAmount::zero(), expiration),
    )
    .unwrap();

    rt.set_epoch(201);
    h.change_beneficiary(
        &mut rt,
        h.owner,
        BeneficiaryChange::new(h.owner, TokenAmount::zero(), ChainEpoch::from(0)),
        Some(h.owner),
    )
    .unwrap();

    let info = h.get_info(&mut rt);
    assert_eq!(None, info.pending_beneficiary_term);
    assert_eq!(h.owner, info.beneficiary);
    assert_eq!(TokenAmount::zero(), info.beneficiary_term.quota);
    assert_eq!(ChainEpoch::from(0), info.beneficiary_term.expiration);
    assert_eq!(TokenAmount::zero(), info.beneficiary_term.used_quota);
    h.check_state(&rt);
}

#[test]
fn successfully_increase_quota() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);
    let beneficiary_term =
        BeneficiaryTerm::new(TokenAmount::from(100), TokenAmount::zero(), ChainEpoch::from(200));
    h.propose_approve_initial_beneficiary(&mut rt, first_beneficiary_id, beneficiary_term.clone())
        .unwrap();

    //increase quota
    let increase_quota = TokenAmount::from(100) + beneficiary_term.quota.clone();
    let increase_beneficiary_change = BeneficiaryChange::new(
        first_beneficiary_id,
        increase_quota.clone(),
        beneficiary_term.expiration,
    );
    h.change_beneficiary(&mut rt, h.owner, increase_beneficiary_change.clone(), None).unwrap();
    let mut info = h.get_info(&mut rt);
    let pending_beneficiary_term = info.pending_beneficiary_term.unwrap();
    assert_eq!(first_beneficiary_id, info.beneficiary);
    assert_eq!(increase_quota, pending_beneficiary_term.new_quota);
    assert_eq!(beneficiary_term.expiration, pending_beneficiary_term.new_expiration);

    //confirm increase quota
    h.change_beneficiary(
        &mut rt,
        first_beneficiary_id,
        increase_beneficiary_change,
        Some(first_beneficiary_id),
    )
    .unwrap();
    info = h.get_info(&mut rt);
    assert_eq!(None, info.pending_beneficiary_term);
    assert_eq!(first_beneficiary_id, info.beneficiary);
    assert_eq!(increase_quota, info.beneficiary_term.quota);
    assert_eq!(beneficiary_term.expiration, info.beneficiary_term.expiration);
    h.check_state(&rt);
}

#[test]
fn fails_approval_message_with_invalidate_params() {
    let (mut h, mut rt) = setup();
    let first_beneficiary_id = Address::new_id(999);

    // proposal beneficiary
    h.change_beneficiary(
        &mut rt,
        h.owner,
        BeneficiaryChange::new(first_beneficiary_id, TokenAmount::from(100), 200),
        None,
    )
    .unwrap();
    assert!(h.get_info(&mut rt).pending_beneficiary_term.is_some());

    //expiration in approval message must equal with proposal
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.change_beneficiary(
            &mut rt,
            first_beneficiary_id,
            BeneficiaryChange::new(first_beneficiary_id, TokenAmount::from(100), 201),
            None,
        ),
    );

    //quota in approval message must equal with proposal
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.change_beneficiary(
            &mut rt,
            first_beneficiary_id,
            BeneficiaryChange::new(first_beneficiary_id, TokenAmount::from(101), 200),
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
            BeneficiaryChange::new(second_beneficiary_id, TokenAmount::from(100), 200),
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
            BeneficiaryChange::new(first_beneficiary_id, TokenAmount::from(100), 200),
            None,
        ),
    );

    //quota must bigger than zero while change beneficiary to address(not owner)
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.change_beneficiary(
            &mut rt,
            h.owner,
            BeneficiaryChange::new(first_beneficiary_id, TokenAmount::from(0), 200),
            None,
        ),
    );

    //quota must be zero while change beneficiary to owner address
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.change_beneficiary(
            &mut rt,
            h.owner,
            BeneficiaryChange::new(h.owner, TokenAmount::from(20), 0),
            None,
        ),
    );

    //expiration must be zero while change beneficiary to owner address
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.change_beneficiary(
            &mut rt,
            h.owner,
            BeneficiaryChange::new(h.owner, TokenAmount::from(0), 1),
            None,
        ),
    );
}

#[test]
fn successfully_get_beneficiary() {
    let (mut h, mut rt) = setup();
    let mut info = h.get_info(&mut rt);
    assert_eq!(h.owner, info.beneficiary);
    assert_eq!(ChainEpoch::from(0), info.beneficiary_term.expiration);
    assert_eq!(TokenAmount::zero(), info.beneficiary_term.quota);
    assert_eq!(TokenAmount::zero(), info.beneficiary_term.used_quota);

    let first_beneficiary_id = Address::new_id(999);
    let beneficiary_term =
        BeneficiaryTerm::new(TokenAmount::from(100), TokenAmount::zero(), ChainEpoch::from(200));
    h.propose_approve_initial_beneficiary(&mut rt, first_beneficiary_id, beneficiary_term.clone())
        .unwrap();

    info = h.get_info(&mut rt);
    assert_eq!(first_beneficiary_id, info.beneficiary);
    assert_eq!(beneficiary_term.expiration, info.beneficiary_term.expiration);
    assert_eq!(beneficiary_term.quota, info.beneficiary_term.quota);
    assert_eq!(TokenAmount::zero(), info.beneficiary_term.used_quota);

    let withdraw_fund = TokenAmount::from(40);
    let left_quota = TokenAmount::from(60);
    h.withdraw_funds(&mut rt, h.beneficiary, &withdraw_fund, &withdraw_fund, &TokenAmount::zero())
        .unwrap();

    info = h.get_info(&mut rt);
    assert_eq!(first_beneficiary_id, info.beneficiary);
    assert_eq!(beneficiary_term.expiration, info.beneficiary_term.expiration);
    assert_eq!(left_quota, info.beneficiary_term.available(rt.epoch));
    assert_eq!(withdraw_fund, info.beneficiary_term.used_quota);
}
