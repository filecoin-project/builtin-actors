use fil_actor_miner_state_v9::{
    ActiveBeneficiary, ChangeBeneficiaryParams, Method as MinerMethod, PendingBeneficiaryChange,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::RegisteredSealProof;
use test_vm::util::{
    apply_code, change_beneficiary, create_accounts, create_miner, get_beneficiary,
    withdraw_balance,
};
use test_vm::VM;

#[test]
fn change_beneficiary_success() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 4, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, beneficiary, another_beneficiary, query_addr) =
        (addrs[0], addrs[0], addrs[1], addrs[2], addrs[3]);

    // create miner
    let miner_id = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from_whole(1_000),
    )
    .0;

    //change from owner to beneficiary address
    let beneficiary_change_proposal =
        ChangeBeneficiaryParams::new(beneficiary, TokenAmount::from_atto(100), 500);
    change_beneficiary(&v, owner, miner_id, &beneficiary_change_proposal);
    let mut get_beneficiary_return = get_beneficiary(&v, query_addr, miner_id);
    let pending_beneficiary_term = get_beneficiary_return.proposed.unwrap();
    assert_pending(&beneficiary_change_proposal, &pending_beneficiary_term);
    assert!(pending_beneficiary_term.approved_by_beneficiary);
    assert!(!pending_beneficiary_term.approved_by_nominee);

    change_beneficiary(&v, beneficiary, miner_id, &beneficiary_change_proposal);
    get_beneficiary_return = get_beneficiary(&v, query_addr, miner_id);
    assert!(get_beneficiary_return.proposed.is_none());
    assert_active(&beneficiary_change_proposal, &get_beneficiary_return.active);

    //change beneficiary to another address
    let change_another_beneificiary_proposal =
        ChangeBeneficiaryParams::new(another_beneficiary, TokenAmount::from_atto(100), 500);
    change_beneficiary(&v, owner, miner_id, &change_another_beneificiary_proposal);
    let mut get_beneficiary_return = get_beneficiary(&v, query_addr, miner_id);
    let mut pending_beneficiary_term = get_beneficiary_return.proposed.unwrap();
    assert_pending(&change_another_beneificiary_proposal, &pending_beneficiary_term);
    assert!(!pending_beneficiary_term.approved_by_beneficiary);
    assert!(!pending_beneficiary_term.approved_by_nominee);

    change_beneficiary(&v, another_beneficiary, miner_id, &change_another_beneificiary_proposal);
    get_beneficiary_return = get_beneficiary(&v, query_addr, miner_id);
    pending_beneficiary_term = get_beneficiary_return.proposed.unwrap();
    assert_pending(&change_another_beneificiary_proposal, &pending_beneficiary_term);
    assert!(!pending_beneficiary_term.approved_by_beneficiary);
    assert!(pending_beneficiary_term.approved_by_nominee);

    change_beneficiary(&v, beneficiary, miner_id, &change_another_beneificiary_proposal);
    get_beneficiary_return = get_beneficiary(&v, query_addr, miner_id);
    assert!(get_beneficiary_return.proposed.is_none());
    assert_active(&change_another_beneificiary_proposal, &get_beneficiary_return.active);
    v.assert_state_invariants();
}

#[test]
fn change_beneficiary_back_owner_success() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, beneficiary, query_addr) = (addrs[0], addrs[0], addrs[1], addrs[2]);

    // create miner
    let miner_id = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from_whole(1_000),
    )
    .0;

    let quota = TokenAmount::from_atto(100);
    let beneficiary_change_proposal = ChangeBeneficiaryParams::new(beneficiary, quota.clone(), 500);
    change_beneficiary(&v, owner, miner_id, &beneficiary_change_proposal);
    change_beneficiary(&v, beneficiary, miner_id, &beneficiary_change_proposal);

    //case1 need beneficiary(non-main) to confirm
    let back_owner_proposal = ChangeBeneficiaryParams::new(owner, TokenAmount::zero(), 0);
    change_beneficiary(&v, owner, miner_id, &back_owner_proposal);
    change_beneficiary(&v, beneficiary, miner_id, &back_owner_proposal);

    let get_beneficiary_return = get_beneficiary(&v, query_addr, miner_id);
    assert!(get_beneficiary_return.proposed.is_none());
    assert_active(&back_owner_proposal, &get_beneficiary_return.active);

    //case2 beneficiary(non-main) used up
    change_beneficiary(&v, owner, miner_id, &beneficiary_change_proposal);
    change_beneficiary(&v, beneficiary, miner_id, &beneficiary_change_proposal);
    withdraw_balance(&v, beneficiary, miner_id, quota.clone(), quota);

    change_beneficiary(&v, owner, miner_id, &back_owner_proposal);
    let get_beneficiary_return = get_beneficiary(&v, query_addr, miner_id);
    assert!(get_beneficiary_return.proposed.is_none());
    assert_active(&back_owner_proposal, &get_beneficiary_return.active);

    //case2 beneficiary(non-main) expiration
    change_beneficiary(&v, owner, miner_id, &beneficiary_change_proposal);
    change_beneficiary(&v, beneficiary, miner_id, &beneficiary_change_proposal);

    let v = v.with_epoch(1000);
    change_beneficiary(&v, owner, miner_id, &back_owner_proposal);
    let get_beneficiary_return = get_beneficiary(&v, query_addr, miner_id);
    assert!(get_beneficiary_return.proposed.is_none());
    assert_active(&back_owner_proposal, &get_beneficiary_return.active);
}

#[test]
fn change_beneficiary_fail() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, beneficiary, addr) = (addrs[0], addrs[0], addrs[1], addrs[2]);

    // create miner
    let miner_id = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from_whole(1_000),
    )
    .0;

    apply_code(
        &v,
        addr,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ChangeBeneficiary as u64,
        ChangeBeneficiaryParams::new(beneficiary, TokenAmount::from_atto(0), 0),
        ExitCode::USR_FORBIDDEN,
    );

    let beneficiary_change_proposal =
        ChangeBeneficiaryParams::new(beneficiary, TokenAmount::from_atto(100), 500);
    change_beneficiary(&v, owner, miner_id, &beneficiary_change_proposal);

    //argument not match with pending
    apply_code(
        &v,
        beneficiary,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ChangeBeneficiary as u64,
        ChangeBeneficiaryParams::new(beneficiary, TokenAmount::from_atto(100), 400),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    apply_code(
        &v,
        beneficiary,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ChangeBeneficiary as u64,
        ChangeBeneficiaryParams::new(beneficiary, TokenAmount::from_atto(80), 500),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    apply_code(
        &v,
        beneficiary,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ChangeBeneficiary as u64,
        ChangeBeneficiaryParams::new(addr, TokenAmount::from_atto(80), 500),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    //message from must be owner/beneficiary/new beneficiary
    apply_code(
        &v,
        addr,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ChangeBeneficiary as u64,
        beneficiary_change_proposal.clone(),
        ExitCode::USR_FORBIDDEN,
    );
    change_beneficiary(&v, beneficiary, miner_id, &beneficiary_change_proposal);

    //change back to owner, quota and expiration must be zero
    apply_code(
        &v,
        owner,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ChangeBeneficiary as u64,
        ChangeBeneficiaryParams::new(owner, TokenAmount::from_atto(80), 0),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
    apply_code(
        &v,
        owner,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ChangeBeneficiary as u64,
        ChangeBeneficiaryParams::new(owner, TokenAmount::from_atto(0), 100),
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    //success change back to owner
    let back_owner_proposal = ChangeBeneficiaryParams::new(owner, TokenAmount::zero(), 0);
    change_beneficiary(&v, owner, miner_id, &back_owner_proposal);
    change_beneficiary(&v, beneficiary, miner_id, &back_owner_proposal);
    v.assert_state_invariants();
}

fn assert_pending(
    beneficiary_change_proposal: &ChangeBeneficiaryParams,
    pending_beneficiary_term: &PendingBeneficiaryChange,
) {
    assert_eq!(
        beneficiary_change_proposal.new_beneficiary,
        pending_beneficiary_term.new_beneficiary
    );
    assert_eq!(beneficiary_change_proposal.new_quota, pending_beneficiary_term.new_quota);
    assert_eq!(beneficiary_change_proposal.new_expiration, pending_beneficiary_term.new_expiration);
}

fn assert_active(
    beneficiary_change_proposal: &ChangeBeneficiaryParams,
    active_beneficiary: &ActiveBeneficiary,
) {
    assert_eq!(beneficiary_change_proposal.new_beneficiary, active_beneficiary.beneficiary);
    assert_eq!(beneficiary_change_proposal.new_quota, active_beneficiary.term.quota);
    assert_eq!(beneficiary_change_proposal.new_expiration, active_beneficiary.term.expiration);
}
