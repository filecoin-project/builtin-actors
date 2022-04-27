use fil_actor_multisig::{
    Actor as MultisigActor, ConstructorParams, Method, State, Transaction, TxnID, SIGNERS_MAX,
};
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{INIT_ACTOR_ADDR, SYSTEM_ACTOR_ADDR};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::{Address, BLS_PUB_LEN};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;

mod util;

fn construct_runtime(receiver: Address) -> MockRuntime {
    MockRuntime {
        receiver,
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

// Constructor

#[test]
fn test_construction_fail_to_construct_multisig_actor_with_0_signers() {
    let msig = Address::new_id(1000);
    let mut rt = construct_runtime(msig);
    let zero_signer_params = ConstructorParams {
        signers: Vec::new(),
        num_approvals_threshold: 1,
        unlock_duration: 1,
        start_epoch: 0,
    };
    rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
    rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);

    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MultisigActor>(
            Method::Constructor as u64,
            &RawBytes::serialize(&zero_signer_params).unwrap(),
        ),
    );
    rt.verify();
}

#[test]
fn test_construction_fail_to_construct_multisig_with_more_than_max_signers() {
    let msig = Address::new_id(1000);
    let mut rt = construct_runtime(msig);
    let mut signers = Vec::new();
    let mut i: u64 = 0;
    while i <= SIGNERS_MAX as u64 {
        signers.push(Address::new_id(i + 1000));
        i += 1;
    }
    let over_max_signers_params = ConstructorParams {
        signers,
        num_approvals_threshold: 1,
        unlock_duration: 1,
        start_epoch: 0,
    };
    rt.expect_validate_caller_addr(vec![*INIT_ACTOR_ADDR]);
    rt.set_caller(*INIT_ACTOR_CODE_ID, *INIT_ACTOR_ADDR);
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        rt.call::<MultisigActor>(
            Method::Constructor as u64,
            &RawBytes::serialize(&over_max_signers_params).unwrap(),
        ),
    );
    rt.verify();
}

// Propose

#[test]
fn test_simple_propose() {
    let msig = Address::new_id(1000);
    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();

    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let no_unlock_duration = 0;
    let start_epoch = 0;
    let signers = vec![anne, bob];

    let send_value = TokenAmount::from(10u8);
    h.construct_and_verify(&mut rt, 2, no_unlock_duration, start_epoch, signers);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
    h.propose_ok(&mut rt, chuck, send_value.clone(), METHOD_SEND, RawBytes::default());
    let txn0 = Transaction {
        to: chuck,
        value: send_value,
        method: METHOD_SEND,
        params: RawBytes::default(),
        approved: vec![anne],
    };
    let expect_txns = vec![(TxnID(0), txn0)];
    h.assert_transactions(&rt, expect_txns);
}

// AddSigner
#[test]
fn test_add_signer() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let chuck_pubkey = Address::new_bls(&[3u8; BLS_PUB_LEN]).unwrap();

    struct TestCase<'a> {
        #[allow(dead_code)]
        desc: &'a str,

        id_addr_mapping: Vec<(Address, Address)>, // non-id to id
        initial_signers: Vec<Address>,
        initial_approvals: u64,

        add_signer: Address,
        increase: bool,

        expect_signers: Vec<Address>,
        expect_approvals: u64,
        code: ExitCode,
    }

    let test_cases = vec![
        TestCase{
            desc: "happy path add signer",
            id_addr_mapping: Vec::new(),
            initial_signers: vec![anne, bob],
            initial_approvals: 2,
            add_signer: chuck,
            increase: false,
            expect_signers: vec![anne, bob, chuck],
            expect_approvals: 2,
            code: ExitCode::OK,
        },
        TestCase{
            desc: "add signer and increase threshold",
            id_addr_mapping: Vec::new(),
            initial_signers: vec![anne, bob],
            initial_approvals: 2,
            add_signer: chuck,
            increase: true,
            expect_signers: vec![anne, bob, chuck],
            expect_approvals: 3,
            code: ExitCode::OK,
        },
        TestCase{
            desc: "fail to add signer that already exists",
            id_addr_mapping: Vec::new(),
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 2,
            add_signer: chuck,
            increase: false,
            expect_signers: vec![anne, bob, chuck],
            expect_approvals: 3,
            code: ExitCode::USR_FORBIDDEN,
        },
        TestCase{
            desc: "fail to add signer with ID address that already exists even thugh we only have non ID address as approver",
            id_addr_mapping: vec![(chuck_pubkey, chuck)],
            initial_signers: vec![anne, bob, chuck_pubkey],
            initial_approvals: 3,
            add_signer: chuck,
            increase:false,
            expect_signers: vec![anne, bob, chuck],
            expect_approvals: 3,
            code: ExitCode::USR_FORBIDDEN,
        },
        TestCase{
            desc: "fail to add signer with ID address that already exists even thugh we only have non ID address as approver",
            id_addr_mapping: vec![(chuck_pubkey, chuck)],
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 3,
            add_signer: chuck_pubkey,
            increase:false,
            expect_signers: vec![anne, bob, chuck],
            expect_approvals: 3,
            code: ExitCode::USR_FORBIDDEN,
        }
    ];

    for tc in test_cases {
        let mut rt = construct_runtime(msig);
        let h = util::ActorHarness::new();
        for (src, target) in tc.id_addr_mapping {
            rt.id_addresses.insert(src, target);
        }

        h.construct_and_verify(&mut rt, tc.initial_approvals, 0, 0, tc.initial_signers);

        rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
        match tc.code {
            ExitCode::OK => {
                let ret = h.add_signer(&mut rt, tc.add_signer, tc.increase).unwrap();
                assert_eq!(RawBytes::default(), ret);
                let st: State = rt.get_state();
                assert_eq!(tc.expect_signers, st.signers);
                assert_eq!(tc.expect_approvals, st.num_approvals_threshold);
            }
            _ => expect_abort(tc.code, h.add_signer(&mut rt, tc.add_signer, tc.increase)),
        }
    }
}

// RemoveSigner

#[test]
fn test_remove_signer() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let anne_non_id = Address::new_bls(&[3u8; BLS_PUB_LEN]).unwrap();
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);
    let richard = Address::new_id(104);

    struct TestCase<'a> {
        #[allow(dead_code)]
        desc: &'a str,

        initial_signers: Vec<Address>,
        initial_approvals: u64,

        remove_signer: Address,
        decrease: bool,

        expect_signers: Vec<Address>,
        expect_approvals: u64,
        code: ExitCode,
    }

    let test_cases = vec![
        TestCase {
            desc: "happy path remove signer",
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 2,
            remove_signer: chuck,
            decrease: false,
            expect_signers: vec![anne, bob],
            expect_approvals: 2,
            code: ExitCode::OK,
        },
        TestCase {
            desc: "remove signer and decrease threshold",
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 2,
            remove_signer: chuck,
            decrease: true,
            expect_signers: vec![anne, bob],
            expect_approvals: 1,
            code: ExitCode::OK,
        },
        TestCase {
            desc: "remove signer when msig is created with an id addr and removed with pk addr",
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 2,
            remove_signer: anne_non_id,
            decrease: true,
            expect_signers: vec![bob, chuck],
            expect_approvals: 1,
            code: ExitCode::OK,
        },
        TestCase {
            desc: "remove signer when msig created with pk addr and removed with id addr",
            initial_signers: vec![anne_non_id, bob, chuck],
            initial_approvals: 2,
            remove_signer: anne,
            decrease: true,
            expect_signers: vec![bob, chuck],
            expect_approvals: 1,
            code: ExitCode::OK,
        },
        TestCase {
            desc: "remove signer when msig is created and removed with pk addr",
            initial_signers: vec![anne_non_id, bob, chuck],
            initial_approvals: 2,
            remove_signer: anne_non_id,
            decrease: true,
            expect_signers: vec![bob, chuck],
            expect_approvals: 1,
            code: ExitCode::OK,
        },
        TestCase {
            desc: "fail signer if decrease is set to false and number of signers below threshold",
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 3,
            remove_signer: chuck,
            decrease: false,
            expect_signers: vec![],
            expect_approvals: 0,
            code: ExitCode::USR_ILLEGAL_ARGUMENT,
        },
        TestCase {
            desc: "remove signer from single signer list",
            initial_signers: vec![anne],
            initial_approvals: 1,
            remove_signer: anne,
            decrease: false,
            expect_signers: vec![],
            expect_approvals: 0,
            code: ExitCode::USR_FORBIDDEN,
        },
        TestCase {
            desc: "fail to remove non-signer",
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 2,
            remove_signer: richard,
            decrease: false,
            expect_signers: vec![],
            expect_approvals: 0,
            code: ExitCode::USR_FORBIDDEN,
        },
        TestCase {
            desc: "fail to remove a signer and decrease approvals below 1",
            initial_signers: vec![anne, bob, chuck],
            initial_approvals: 1,
            remove_signer: anne,
            decrease: true,
            expect_signers: vec![anne, bob, chuck],
            expect_approvals: 1,
            code: ExitCode::USR_ILLEGAL_ARGUMENT,
        },
    ];

    for tc in test_cases {
        let mut rt = construct_runtime(msig);
        rt.id_addresses.insert(anne_non_id, anne);
        let h = util::ActorHarness::new();
        h.construct_and_verify(&mut rt, tc.initial_approvals, 0, 0, tc.initial_signers);

        rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
        let ret = h.remove_signer(&mut rt, tc.remove_signer, tc.decrease);

        match tc.code {
            ExitCode::OK => {
                assert_eq!(RawBytes::default(), ret.unwrap());
                let st: State = rt.get_state();
                assert_eq!(tc.expect_signers, st.signers);
                assert_eq!(tc.expect_approvals, st.num_approvals_threshold);
            }
            _ => assert_eq!(
                tc.code,
                ret.expect_err("remove signer return expected to be actor error").exit_code()
            ),
        }
        rt.verify();
    }
}

// SwapSigner
#[test]
fn test_signer_swap() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let bob_non_id = Address::new_bls(&[1u8; BLS_PUB_LEN]).unwrap();
    let chuck = Address::new_id(103);
    let darlene = Address::new_id(104);
    let num_approvals: u64 = 1;

    struct TestCase<'a> {
        #[allow(dead_code)]
        desc: &'a str,

        initial_signers: Vec<Address>,
        swap_to: Address,
        swap_from: Address,
        expect_signers: Vec<Address>,
        code: ExitCode,
    }

    let test_cases = vec![
        TestCase {
            desc: "happy path remove signer",
            initial_signers: vec![anne, bob],
            swap_to: chuck,
            swap_from: bob,
            expect_signers: vec![anne, chuck],
            code: ExitCode::OK,
        },
        TestCase {
            desc: "swap signer when multi-sig is created with it's ID address but we ask for a swap with it's non-ID address",
            initial_signers: vec![anne, bob],
            swap_to: chuck,
            swap_from: bob_non_id,
            expect_signers: vec![anne, chuck],
            code: ExitCode::OK,
        },
        TestCase {
            desc: "swap signer when multi-sig is created with it's non-ID address but we ask for a swap with it's ID address",
            initial_signers: vec![anne, bob_non_id],
            swap_to: chuck,
            swap_from: bob,
            expect_signers: vec![anne, chuck],
            code: ExitCode::OK,
        },
        TestCase {
            desc: "swap signer when multi-sig is created with it's non-ID address and we ask for a swap with it's non-ID address",
            initial_signers: vec![anne, bob_non_id],
            swap_to: chuck,
            swap_from: bob_non_id,
            expect_signers: vec![anne, chuck],
            code: ExitCode::OK,
        },
        TestCase {
            desc: "fail to swap when from signer not found",
            initial_signers: vec![anne, bob],
            swap_to: chuck,
            swap_from: darlene,
            expect_signers: vec![],
            code: ExitCode::USR_FORBIDDEN,
        },
        TestCase {
            desc: "fail to swap when to signer already present",
            initial_signers: vec![anne, bob],
            swap_to: bob,
            swap_from: anne,
            expect_signers: vec![],
            code: ExitCode::USR_ILLEGAL_ARGUMENT,
        },
        TestCase {
            desc: "fail to swap when to signer ID address already present(even though we have the non-ID address)",
            initial_signers: vec![anne, bob_non_id],
            swap_to: bob,
            swap_from: anne,
            expect_signers: vec![],
            code: ExitCode::USR_ILLEGAL_ARGUMENT,
        },
        TestCase {
            desc: "fail to swap when to signer non-ID address already present(even though we have the ID address)",
            initial_signers: vec![anne, bob],
            swap_to: bob_non_id,
            swap_from: anne,
            expect_signers: vec![],
            code: ExitCode::USR_ILLEGAL_ARGUMENT,
        }
    ];

    for tc in test_cases {
        let mut rt = construct_runtime(msig);
        rt.id_addresses.insert(bob_non_id, bob);
        let h = util::ActorHarness::new();
        h.construct_and_verify(&mut rt, num_approvals, 0, 0, tc.initial_signers);

        rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
        let ret = h.swap_signers(&mut rt, tc.swap_from, tc.swap_to);
        match tc.code {
            ExitCode::OK => {
                assert_eq!(RawBytes::default(), ret.unwrap());
                let st: State = rt.get_state();
                assert_eq!(tc.expect_signers, st.signers);
            }
            _ => assert_eq!(
                tc.code,
                ret.expect_err("swap signer return expected to be actor error").exit_code()
            ),
        };
    }
}

// Approve

#[test]
fn test_approve_simple_propose_and_approval() {
    // setup rt
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);

    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();
    // construct msig
    let signers = vec![anne, bob];

    h.construct_and_verify(&mut rt, 2, 0, 0, signers);

    let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
    let fake_method = 42;
    let fake_ret = RawBytes::from(vec![4, 3, 2, 1]);
    let send_value = TokenAmount::from(10u8);
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
    let proposal_hash =
        h.propose_ok(&mut rt, chuck, send_value.clone(), fake_method, fake_params.clone());

    // assert txn
    let expect_txn = Transaction {
        to: chuck,
        value: send_value.clone(),
        method: fake_method,
        params: fake_params.clone(),
        approved: vec![anne],
    };
    h.assert_transactions(&rt, vec![(TxnID(0), expect_txn)]);

    // approval
    rt.set_balance(send_value.clone());
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, bob);
    rt.expect_send(chuck, fake_method, fake_params, send_value, fake_ret, ExitCode::OK);
    h.approve_ok(&mut rt, TxnID(0), proposal_hash);
    h.assert_transactions(&rt, vec![]);
}

// Cancel
#[test]
fn test_simple_propose_and_cancel() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);

    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();
    let signers = vec![anne, bob];

    h.construct_and_verify(&mut rt, 2, 0, 0, signers);

    let fake_params = RawBytes::from(vec![1, 2, 3, 4]);
    let fake_method = 42;
    let send_value = TokenAmount::from(10u8);
    // anne proposes tx
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, anne);
    let proposal_hash = h.propose_ok(&mut rt, chuck, send_value, fake_method, fake_params);

    // anne cancels the tx
    let ret = h.cancel(&mut rt, TxnID(0), proposal_hash).unwrap();
    assert_eq!(RawBytes::default(), ret);

    // tx should be removed from actor state
    h.assert_transactions(&rt, vec![]);
}

// LockBalance
#[test]
fn test_lock_balance_checks_preconditions() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);

    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();

    h.construct_and_verify(&mut rt, 1, 0, 0, vec![anne]);

    let vest_start = 0_i64;
    let lock_amount = TokenAmount::from(100_000u32);
    let vest_duration = 1000_i64;

    // Disallow negative duration but allow negative start epoch
    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.lock_balance(&mut rt, vest_start, -1_i64, lock_amount),
    );

    // Disallow negative amount
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.lock_balance(&mut rt, vest_start, vest_duration, TokenAmount::from(-1i32)),
    );
}

// ChangeNumApprovalsThreshold
#[test]
fn test_change_threshold_happy_path_decrease_threshold() {
    let msig = Address::new_id(100);
    let anne = Address::new_id(101);
    let bob = Address::new_id(102);
    let chuck = Address::new_id(103);

    let mut rt = construct_runtime(msig);
    let h = util::ActorHarness::new();
    let signers = vec![anne, bob, chuck];
    let initial_threshold = 2;

    h.construct_and_verify(&mut rt, initial_threshold, 0, 0, signers);

    rt.set_caller(*MULTISIG_ACTOR_CODE_ID, msig);
    let ret = h.change_num_approvals_threshold(&mut rt, 1).unwrap();
    assert_eq!(RawBytes::default(), ret);
    let st: State = rt.get_state();
    assert_eq!(1, st.num_approvals_threshold);
}
