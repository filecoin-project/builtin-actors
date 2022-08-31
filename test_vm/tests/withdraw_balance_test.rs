use fil_actor_miner::{ChangeBeneficiaryParams, Method as MinerMethod, WithdrawBalanceParams};
use fil_actors_runtime::cbor::serialize;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::RegisteredSealProof;
use fvm_shared::METHOD_SEND;
use test_vm::util::{
    apply_code, change_beneficiary, create_accounts, create_miner, withdraw_balance,
};
use test_vm::{ExpectInvocation, VM};

#[test]
fn withdraw_balance_success() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 2, TokenAmount::from(10_000e18 as i128));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, beneficiary) = (addrs[0], addrs[0], addrs[1]);

    // create miner
    let miner_id = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(1_000e18 as i128),
    )
    .0;

    //withdraw from owner
    let mut withdraw_amount = TokenAmount::from(100);
    let mut withdraw_return = withdraw_balance(&v, owner, miner_id, withdraw_amount.clone());

    let mut withdraw_balance_params_se = serialize(
        &WithdrawBalanceParams { amount_requested: withdraw_amount.clone() },
        "withdraw  balance params",
    )
    .unwrap();
    ExpectInvocation {
        to: miner_id,
        method: MinerMethod::WithdrawBalance as u64,
        params: Some(withdraw_balance_params_se),
        subinvocs: Some(vec![ExpectInvocation {
            to: owner,
            method: METHOD_SEND as u64,
            value: Some(withdraw_amount.clone()),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
    assert_eq!(withdraw_amount, withdraw_return.amount_withdrawn);

    //withdraw from beneficiary
    let quota = TokenAmount::from(100);
    let beneficiary_change_proposal = ChangeBeneficiaryParams::new(beneficiary, quota, 500);
    change_beneficiary(&v, owner, miner_id, &beneficiary_change_proposal);
    change_beneficiary(&v, beneficiary, miner_id, &beneficiary_change_proposal);

    //withdraw 40 by owner
    withdraw_amount = TokenAmount::from(40);
    withdraw_return = withdraw_balance(&v, owner, miner_id, withdraw_amount.clone());
    withdraw_balance_params_se = serialize(
        &WithdrawBalanceParams { amount_requested: withdraw_amount.clone() },
        "withdraw  balance params",
    )
    .unwrap();
    ExpectInvocation {
        to: miner_id,
        method: MinerMethod::WithdrawBalance as u64,
        params: Some(withdraw_balance_params_se),
        subinvocs: Some(vec![ExpectInvocation {
            to: beneficiary,
            method: METHOD_SEND as u64,
            value: Some(withdraw_amount.clone()),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
    assert_eq!(withdraw_amount, withdraw_return.amount_withdrawn);

    //withdraw left by beneficiary
    let left_amount = TokenAmount::from(60);
    withdraw_amount = TokenAmount::from(100);
    withdraw_return = withdraw_balance(&v, beneficiary, miner_id, withdraw_amount.clone());
    withdraw_balance_params_se = serialize(
        &WithdrawBalanceParams { amount_requested: withdraw_amount },
        "withdraw  balance params",
    )
    .unwrap();
    ExpectInvocation {
        to: miner_id,
        method: MinerMethod::WithdrawBalance as u64,
        params: Some(withdraw_balance_params_se),
        subinvocs: Some(vec![ExpectInvocation {
            to: beneficiary,
            method: METHOD_SEND as u64,
            value: Some(left_amount.clone()),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
    assert_eq!(left_amount, withdraw_return.amount_withdrawn);
}

#[test]
fn withdraw_balance_fail() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, TokenAmount::from(10_000e18 as i128));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (owner, worker, beneficiary, addr) = (addrs[0], addrs[0], addrs[1], addrs[2]);

    // create miner
    let miner_id = create_miner(
        &mut v,
        owner,
        worker,
        seal_proof.registered_window_post_proof().unwrap(),
        TokenAmount::from(1_000e18 as i128),
    )
    .0;

    //set as beneficiary
    let quota = TokenAmount::from(100);
    let beneficiary_change_proposal = ChangeBeneficiaryParams::new(beneficiary, quota, 500);
    change_beneficiary(&v, owner, miner_id, &beneficiary_change_proposal);

    let withdraw_amount = TokenAmount::from(100);
    //message from must be owner
    apply_code(
        &v,
        beneficiary,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::WithdrawBalance as u64,
        WithdrawBalanceParams { amount_requested: withdraw_amount.clone() },
        ExitCode::USR_FORBIDDEN,
    );
    let withdraw_balance_params_se = serialize(
        &WithdrawBalanceParams { amount_requested: withdraw_amount },
        "withdraw  balance params",
    )
    .unwrap();

    ExpectInvocation {
        to: miner_id,
        method: MinerMethod::WithdrawBalance as u64,
        params: Some(withdraw_balance_params_se),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    //confirm beneficiary
    change_beneficiary(&v, beneficiary, miner_id, &beneficiary_change_proposal);

    let withdraw_amount = TokenAmount::from(50);
    //message from must be beneficiary or owner
    apply_code(
        &v,
        addr,
        miner_id,
        TokenAmount::zero(),
        MinerMethod::WithdrawBalance as u64,
        WithdrawBalanceParams { amount_requested: withdraw_amount.clone() },
        ExitCode::USR_FORBIDDEN,
    );
    let withdraw_balance_params_se = serialize(
        &WithdrawBalanceParams { amount_requested: withdraw_amount.clone() },
        "withdraw  balance params",
    )
    .unwrap();

    ExpectInvocation {
        to: miner_id,
        method: MinerMethod::WithdrawBalance as u64,
        params: Some(withdraw_balance_params_se.clone()),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    withdraw_balance(&v, beneficiary, miner_id, withdraw_amount.clone());
    ExpectInvocation {
        to: miner_id,
        method: MinerMethod::WithdrawBalance as u64,
        params: Some(withdraw_balance_params_se),
        subinvocs: Some(vec![ExpectInvocation {
            to: beneficiary,
            method: METHOD_SEND as u64,
            value: Some(withdraw_amount),
            ..Default::default()
        }]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
}
