use fil_actor_init::Method as InitMethod;
use fil_actor_init::{Actor as InitActor, ExecReturn, State as InitState};
use fil_actor_miner::{Method as MinerMethod, MinerConstructorParams};
use fil_actor_multisig::{
    compute_proposal_hash, Method as MsigMethod, ProposeParams, Transaction, TxnID, TxnIDParams,
};
use fil_actor_power::{CreateMinerParams, Method as PowerMethod};
use fil_actors_runtime::cbor::serialize;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{INIT_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{BytesDe, RawBytes};
use fvm_shared::address::Address;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::RegisteredPoStProof;
use fvm_shared::METHOD_SEND;
use test_vm::util::{apply_code, apply_ok, create_accounts};
use test_vm::{ExpectInvocation, FIRST_TEST_USER_ADDR, TEST_FAUCET_ADDR, VM};

#[test]
fn test_proposal_hash() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new_with_singletons(&store);
    let addrs = create_accounts(&v, 3, TokenAmount::from(10_000e18 as u64));
    let alice = addrs[0];
    let bob = addrs[1];
    let sys_act_start_bal = v.get_actor(*SYSTEM_ACTOR_ADDR).unwrap().balance;

    // create msig
    let msig_ctor_params = serialize(
        &fil_actor_multisig::ConstructorParams {
            signers: addrs.clone(),
            num_approvals_threshold: 2,
            unlock_duration: 0,
            start_epoch: 0,
        },
        "multisig ctor params",
    )
    .unwrap();
    let msig_ctor_ret: ExecReturn = apply_ok(
        &v,
        alice,
        *INIT_ACTOR_ADDR,
        TokenAmount::from(0 as u64),
        fil_actor_init::Method::Exec as u64,
        fil_actor_init::ExecParams {
            code_cid: *MULTISIG_ACTOR_CODE_ID,
            constructor_params: msig_ctor_params,
        },
    )
    .deserialize()
    .unwrap();
    let msig_addr = msig_ctor_ret.id_address;

    // fund msig and propose send funds to system actor
    let fil_delta = TokenAmount::from(3 * 1_000_000_000 as u64); // 3 nFIL
    let propose_send_sys_params = ProposeParams {
        to: *SYSTEM_ACTOR_ADDR,
        value: fil_delta.clone(),
        method: METHOD_SEND,
        params: RawBytes::default(),
    };
    apply_ok(
        &v,
        alice,
        msig_addr,
        fil_delta.clone(),
        MsigMethod::Propose as u64,
        propose_send_sys_params,
    );

    let wrong_tx = Transaction {
        to: *SYSTEM_ACTOR_ADDR,
        value: fil_delta.clone() - 1 as u64, // incorrect send amount not consistent with proposal
        method: METHOD_SEND,
        approved: vec![alice],
        params: RawBytes::default(),
    };
    let wrong_hash = compute_proposal_hash(&wrong_tx, &v).unwrap();
    let wrong_approval_params = TxnIDParams { id: TxnID(0), proposal_hash: wrong_hash.to_vec() };
    apply_code(
        &v,
        bob,
        msig_addr,
        TokenAmount::zero(),
        MsigMethod::Approve as u64,
        wrong_approval_params,
        ExitCode::USR_ILLEGAL_ARGUMENT,
    );

    let correct_tx = Transaction {
        to: *SYSTEM_ACTOR_ADDR,
        value: fil_delta.clone(),
        method: METHOD_SEND,
        approved: vec![alice],
        params: RawBytes::default(),
    };
    let correct_hash = compute_proposal_hash(&correct_tx, &v).unwrap();
    let approve_params = TxnIDParams { id: TxnID(0), proposal_hash: correct_hash.to_vec() };
    apply_ok(&v, bob, msig_addr, TokenAmount::zero(), MsigMethod::Approve as u64, approve_params);

    assert_eq!(sys_act_start_bal + fil_delta, v.get_actor(*SYSTEM_ACTOR_ADDR).unwrap().balance);
}
