use fil_actor_miner_state_v9::{ChangeBeneficiaryParams, Method as MinerMethod};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::RegisteredSealProof;
use test_vm::util::{
    apply_code, change_beneficiary, change_owner_address, create_accounts, create_miner,
    get_beneficiary,
};
use test_vm::VM;

#[test]
fn change_owner_success() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, new_owner, beneficiary) = (addrs[0], addrs[0], addrs[1], addrs[2]);

    // create miner
    let miner_id = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from_whole(1_000),
    )
    .0;

    change_beneficiary(
        &v,
        owner,
        miner_id,
        &ChangeBeneficiaryParams::new(beneficiary, TokenAmount::from_atto(100), 100),
    );
    change_owner_address(&v, owner, miner_id, new_owner);
    let miner_info = v.get_miner_info(miner_id);
    assert_eq!(new_owner, miner_info.pending_owner_address.unwrap());

    change_owner_address(&v, new_owner, miner_id, new_owner);
    let miner_info = v.get_miner_info(miner_id);
    assert!(miner_info.pending_owner_address.is_none());
    assert_eq!(new_owner, miner_info.owner);
    assert_eq!(new_owner, miner_info.beneficiary);

    v.assert_state_invariants();
}

#[test]
fn keep_beneficiary_when_owner_changed() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, new_owner, beneficiary) = (addrs[0], addrs[0], addrs[1], addrs[2]);

    // create miner
    let miner_id = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from_whole(1_000),
    )
    .0;

    change_beneficiary(
        &v,
        owner,
        miner_id,
        &ChangeBeneficiaryParams::new(beneficiary, TokenAmount::from_atto(100), 100),
    );
    change_beneficiary(
        &v,
        beneficiary,
        miner_id,
        &ChangeBeneficiaryParams::new(beneficiary, TokenAmount::from_atto(100), 100),
    );
    assert_eq!(beneficiary, get_beneficiary(&v, worker, miner_id).active.beneficiary);

    change_owner_address(&v, owner, miner_id, new_owner);
    change_owner_address(&v, new_owner, miner_id, new_owner);
    let miner_info = v.get_miner_info(miner_id);
    assert!(miner_info.pending_owner_address.is_none());
    assert_eq!(new_owner, miner_info.owner);
    assert_eq!(beneficiary, miner_info.beneficiary);

    v.assert_state_invariants();
}

#[test]
fn change_owner_fail() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 4, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, new_owner, addr) = (addrs[0], addrs[0], addrs[1], addrs[2]);

    // create miner
    let miner_id = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from_whole(1_000),
    )
    .0;

    // only owner can proposal
    apply_code(
        &v,
        addr,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ChangeOwnerAddress as u64,
        new_owner,
        ExitCode::USR_FORBIDDEN,
    );

    change_owner_address(&v, owner, miner_id, new_owner);
    // proposal must be the same
    apply_code(
        &v,
        new_owner,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ChangeOwnerAddress as u64,
        addr,
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );
    // only pending can confirm
    apply_code(
        &v,
        addr,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ChangeOwnerAddress as u64,
        new_owner,
        ExitCode::USR_FORBIDDEN,
    );
    //only miner can change proposal
    apply_code(
        &v,
        addr,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::ChangeOwnerAddress as u64,
        addr,
        ExitCode::USR_FORBIDDEN,
    );

    //miner change proposal
    change_owner_address(&v, owner, miner_id, addr);
    //confirm owner proposal
    change_owner_address(&v, addr, miner_id, addr);
    let miner_info = v.get_miner_info(miner_id);
    assert!(miner_info.pending_owner_address.is_none());
    assert_eq!(addr, miner_info.owner);
    assert_eq!(addr, miner_info.beneficiary);

    v.assert_state_invariants();
}
