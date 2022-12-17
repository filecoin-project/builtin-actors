use fil_actor_miner::{ChangeBeneficiaryParams, Method as MinerMethod, WithdrawBalanceParams};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::RegisteredSealProof;
use test_vm::util::{
    apply_code, change_beneficiary, create_accounts, create_miner, withdraw_balance,
};
use test_vm::VM;

#[test]
fn withdraw_balance_success() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 2, TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, beneficiary) = (addrs[0], addrs[0], addrs[1]);

    // create miner
    let miner_id = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from_whole(1_000),
    )
    .0;

    //withdraw from owner
    let withdraw_amount = TokenAmount::from_atto(100);
    withdraw_balance(&v, owner, miner_id, withdraw_amount.clone(), withdraw_amount);

    //withdraw from beneficiary
    let quota = TokenAmount::from_atto(100);
    let beneficiary_change_proposal = ChangeBeneficiaryParams::new(beneficiary, quota, 500);
    change_beneficiary(&v, owner, miner_id, &beneficiary_change_proposal);
    change_beneficiary(&v, beneficiary, miner_id, &beneficiary_change_proposal);

    //withdraw 40 by owner
    let take_first_part = TokenAmount::from_atto(40);
    withdraw_balance(&v, beneficiary, miner_id, take_first_part.clone(), take_first_part);

    //withdraw left by beneficiary
    withdraw_balance(
        &v,
        beneficiary,
        miner_id,
        TokenAmount::from_atto(100),
        TokenAmount::from_atto(60),
    );
}

#[test]
fn withdraw_balance_fail() {
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

    //set as beneficiary
    let quota = TokenAmount::from_atto(100);
    let beneficiary_change_proposal = ChangeBeneficiaryParams::new(beneficiary, quota, 500);
    change_beneficiary(&v, owner, miner_id, &beneficiary_change_proposal);

    //withdraw fail when from is unconfirmed beneficiary
    let withdraw_amount = TokenAmount::from_atto(100);
    let mut balance_before_withdraw = v.get_actor(beneficiary).unwrap().balance;
    apply_code(
        &v,
        beneficiary,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::WithdrawBalance as u64,
        Some(WithdrawBalanceParams { amount_requested: withdraw_amount }),
        ExitCode::USR_FORBIDDEN,
    );
    assert_eq!(balance_before_withdraw, v.get_actor(beneficiary).unwrap().balance);

    //confirm beneficiary
    change_beneficiary(&v, beneficiary, miner_id, &beneficiary_change_proposal);

    //withdraw fail when from address is not owner or beneficiary
    let withdraw_amount = TokenAmount::from_atto(50);
    balance_before_withdraw = v.get_actor(addr).unwrap().balance;
    apply_code(
        &v,
        addr,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::WithdrawBalance as u64,
        Some(WithdrawBalanceParams { amount_requested: withdraw_amount.clone() }),
        ExitCode::USR_FORBIDDEN,
    );
    assert_eq!(balance_before_withdraw, v.get_actor(addr).unwrap().balance);

    //confirm beneficiary is okay
    withdraw_balance(&v, beneficiary, miner_id, withdraw_amount.clone(), withdraw_amount);
}
