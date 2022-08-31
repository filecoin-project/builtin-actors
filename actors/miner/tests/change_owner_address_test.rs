use fil_actors_runtime::test_utils::{
    expect_abort, new_bls_addr, MockRuntime, ACCOUNT_ACTOR_CODE_ID, MULTISIG_ACTOR_CODE_ID,
};
use fvm_shared::{address::Address, error::ExitCode};

mod util;
use util::*;

const NEW_ADDRESS: Address = Address::new_id(1001);
const OTHER_ADDRESS: Address = Address::new_id(1002);

fn setup() -> (ActorHarness, MockRuntime) {
    let period_offset = 100;

    let h = ActorHarness::new(period_offset);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);
    rt.balance.replace(BIG_BALANCE.clone());

    (h, rt)
}

#[test]
fn successful_change() {
    let (mut h, mut rt) = setup();

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, OTHER_ADDRESS);
    h.change_beneficiary(
        &mut rt,
        h.owner,
        &BeneficiaryChange::new(OTHER_ADDRESS, TokenAmount::from(100), 100),
        None,
    )
    .unwrap();

    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, h.owner);
    h.change_owner_address(&mut rt, NEW_ADDRESS).unwrap();

    let info = h.get_info(&rt);
    assert_eq!(h.owner, info.owner);
    assert_eq!(NEW_ADDRESS, info.pending_owner_address.unwrap());

    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, NEW_ADDRESS);
    h.change_owner_address(&mut rt, NEW_ADDRESS).unwrap();

    let info = h.get_info(&rt);
    assert_eq!(NEW_ADDRESS, info.owner);
    assert_eq!(NEW_ADDRESS, info.beneficiary);
    assert!(info.pending_owner_address.is_none());

    h.check_state(&rt);
}

#[test]
fn successful_keep_beneficiary_when_change_owner() {
    let (mut h, mut rt) = setup();

    h.change_beneficiary(
        &mut rt,
        h.owner,
        &BeneficiaryChange::new(OTHER_ADDRESS, TokenAmount::from(100), 100),
        None,
    )
    .unwrap();
    h.change_beneficiary(
        &mut rt,
        OTHER_ADDRESS,
        &BeneficiaryChange::new(OTHER_ADDRESS, TokenAmount::from(100), 100),
        None,
    )
    .unwrap();

    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, h.owner);
    h.change_owner_address(&mut rt, NEW_ADDRESS).unwrap();
    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, NEW_ADDRESS);
    h.change_owner_address(&mut rt, NEW_ADDRESS).unwrap();

    let info = h.get_info(&rt);
    assert_eq!(NEW_ADDRESS, info.owner);
    assert_eq!(OTHER_ADDRESS, info.beneficiary);
    assert!(info.pending_owner_address.is_none());

    h.check_state(&rt);
}

#[test]
fn proposed_must_be_valid() {
    let (h, mut rt) = setup();

    let nominees = vec![
        Address::new_actor(b"Cthulhu"),
        new_bls_addr(42),
        Address::new_secp256k1(&[42; 65]).unwrap(),
    ];

    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, h.owner);

    for nominee in nominees {
        let result = h.change_owner_address(&mut rt, nominee);
        expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);
    }

    h.check_state(&rt);
}

#[test]
fn withdraw_proposal() {
    let (h, mut rt) = setup();
    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, h.owner);
    h.change_owner_address(&mut rt, NEW_ADDRESS).unwrap();

    // revert it
    h.change_owner_address(&mut rt, h.owner).unwrap();

    let info = h.get_info(&rt);
    assert_eq!(h.owner, info.owner);
    assert!(info.pending_owner_address.is_none());

    // new address cannot confirm
    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, NEW_ADDRESS);
    let result = h.change_owner_address(&mut rt, NEW_ADDRESS);
    expect_abort(ExitCode::USR_FORBIDDEN, result);

    h.check_state(&rt);
}

#[test]
fn only_owner_can_propose() {
    let (h, mut rt) = setup();

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    let result = h.change_owner_address(&mut rt, NEW_ADDRESS);
    expect_abort(ExitCode::USR_FORBIDDEN, result);

    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, OTHER_ADDRESS);
    let result = h.change_owner_address(&mut rt, NEW_ADDRESS);
    expect_abort(ExitCode::USR_FORBIDDEN, result);

    h.check_state(&rt);
}

#[test]
fn only_owner_can_change_proposal() {
    let (h, mut rt) = setup();

    // make a proposal
    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, h.owner);
    h.change_owner_address(&mut rt, NEW_ADDRESS).unwrap();

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    let result = h.change_owner_address(&mut rt, OTHER_ADDRESS);
    expect_abort(ExitCode::USR_FORBIDDEN, result);

    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, OTHER_ADDRESS);
    let result = h.change_owner_address(&mut rt, OTHER_ADDRESS);
    expect_abort(ExitCode::USR_FORBIDDEN, result);

    // owner can change it
    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, h.owner);
    h.change_owner_address(&mut rt, OTHER_ADDRESS).unwrap();

    let info = h.get_info(&rt);
    assert_eq!(h.owner, info.owner);
    assert_eq!(OTHER_ADDRESS, info.pending_owner_address.unwrap());

    h.check_state(&rt);
}

#[test]
fn only_nominee_can_confirm() {
    let (h, mut rt) = setup();

    // make a proposal
    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, h.owner);
    h.change_owner_address(&mut rt, NEW_ADDRESS).unwrap();

    // owner re-proposing some address doesn't confirm it
    h.change_owner_address(&mut rt, NEW_ADDRESS).unwrap();
    let info = h.get_info(&rt);
    assert_eq!(h.owner, info.owner);
    assert_eq!(NEW_ADDRESS, info.pending_owner_address.unwrap());

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    let result = h.change_owner_address(&mut rt, OTHER_ADDRESS);
    expect_abort(ExitCode::USR_FORBIDDEN, result);

    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, OTHER_ADDRESS);
    let result = h.change_owner_address(&mut rt, OTHER_ADDRESS);
    expect_abort(ExitCode::USR_FORBIDDEN, result);

    // new address can confirm itself
    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, NEW_ADDRESS);
    h.change_owner_address(&mut rt, NEW_ADDRESS).unwrap();

    let info = h.get_info(&rt);
    assert_eq!(NEW_ADDRESS, info.owner);
    assert!(info.pending_owner_address.is_none());

    h.check_state(&rt);
}

#[test]
fn nominee_must_confirm_self_explicitly() {
    let (h, mut rt) = setup();

    // make a proposal
    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, h.owner);
    h.change_owner_address(&mut rt, NEW_ADDRESS).unwrap();

    // Not own address, should fail
    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, NEW_ADDRESS);
    let result = h.change_owner_address(&mut rt, h.owner);
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);
    let result = h.change_owner_address(&mut rt, OTHER_ADDRESS);
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);

    h.check_state(&rt);
}
