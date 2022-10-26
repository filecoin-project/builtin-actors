use fil_actor_account::State as AccountState;
use fil_actors_runtime::runtime::EMPTY_ARR_CID;
use fil_actors_runtime::test_utils::{
    make_builtin, ACCOUNT_ACTOR_CODE_ID, EMBRYO_ACTOR_CODE_ID, PAYCH_ACTOR_CODE_ID,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;
use num_traits::Zero;
use test_vm::util::pk_addrs_from;
use test_vm::{actor, FIRST_TEST_USER_ADDR, TEST_FAUCET_ADDR, VM};

#[test]
fn state_control() {
    let store = MemoryBlockstore::new();
    let v = VM::new(&store);
    let addr1 = Address::new_id(1000);
    let addr2 = Address::new_id(2222);

    // set actor
<<<<<<< HEAD
    let a1 = actor(
        *ACCOUNT_ACTOR_CODE_ID,
        make_builtin(b"a1-head"),
        42,
        TokenAmount::from_atto(10u8),
        None,
    );
=======
    let a1 =
        actor(*ACCOUNT_ACTOR_CODE_ID, make_builtin(b"a1-head"), 42, TokenAmount::from_atto(10u8));
>>>>>>> master
    v.set_actor(addr1, a1.clone());
    let out = v.get_actor(addr1).unwrap();
    assert_eq!(out, a1);
    let check = v.checkpoint();

<<<<<<< HEAD
    let a2 = actor(
        *PAYCH_ACTOR_CODE_ID,
        make_builtin(b"a2-head"),
        88,
        TokenAmount::from_atto(1u8),
        None,
    );
=======
    let a2 = actor(*PAYCH_ACTOR_CODE_ID, make_builtin(b"a2-head"), 88, TokenAmount::from_atto(1u8));
>>>>>>> master
    v.set_actor(addr2, a2.clone());
    assert_eq!(v.get_actor(addr2).unwrap(), a2);
    // rollback removes a2 but not a1
    v.rollback(check);

    // a2 is gone
    assert_eq!(None, v.get_actor(addr2));
    assert_eq!(v.get_actor(addr1).unwrap(), a1);

    let invariants_check = v.check_state_invariants();
    assert!(invariants_check.is_err());
    assert!(invariants_check.unwrap_err().to_string().contains("AccountState is empty"));
}

fn assert_account_actor(
    exp_call_seq: u64,
    exp_bal: TokenAmount,
    exp_pk_addr: Address,
    v: &VM,
    addr: Address,
) {
    let act = v.get_actor(addr).unwrap();
    let st = v.get_state::<AccountState>(addr).unwrap();
    assert_eq!(exp_call_seq, act.call_seq_num);
    assert_eq!(*ACCOUNT_ACTOR_CODE_ID, act.code);
    assert_eq!(exp_bal, act.balance);
    assert_eq!(exp_pk_addr, st.address);
}

#[test]
fn test_sent() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    // send to uninitialized account actor
    let addr1 = Address::new_bls(&[1; fvm_shared::address::BLS_PUB_LEN]).unwrap();
    v.apply_message(
        TEST_FAUCET_ADDR,
        addr1,
        TokenAmount::from_atto(42u8),
        METHOD_SEND,
        RawBytes::default(),
    )
    .unwrap();
    let expect_id_addr1 = Address::new_id(FIRST_TEST_USER_ADDR);
    assert_account_actor(0, TokenAmount::from_atto(42u8), addr1, &v, expect_id_addr1);

    // send from this account actor to another uninit account actor
    let addr2 = Address::new_bls(&[2; fvm_shared::address::BLS_PUB_LEN]).unwrap();
    v.apply_message(addr1, addr2, TokenAmount::from_atto(41u8), METHOD_SEND, RawBytes::default())
        .unwrap();
    let expect_id_addr2 = Address::new_id(FIRST_TEST_USER_ADDR + 1);
    assert_account_actor(0, TokenAmount::from_atto(41u8), addr2, &v, expect_id_addr2);

    // send between two initialized account actors
    v.apply_message(addr2, addr1, TokenAmount::from_atto(41u8), METHOD_SEND, RawBytes::default())
        .unwrap();
    assert_account_actor(1, TokenAmount::from_atto(42u8), addr1, &v, expect_id_addr1);
    assert_account_actor(1, TokenAmount::zero(), addr2, &v, expect_id_addr2);

    // self send is noop
    v.apply_message(addr1, addr1, TokenAmount::from_atto(1u8), METHOD_SEND, RawBytes::default())
        .unwrap();
    assert_account_actor(2, TokenAmount::from_atto(42u8), addr1, &v, expect_id_addr1);

    // fail with insufficient funds
    let mres = v
        .apply_message(addr2, addr1, TokenAmount::from_atto(1u8), METHOD_SEND, RawBytes::default())
        .unwrap();
    assert_eq!(ExitCode::SYS_INSUFFICIENT_FUNDS, mres.code);
    assert_account_actor(2, TokenAmount::from_atto(42u8), addr1, &v, expect_id_addr1);
    assert_account_actor(2, TokenAmount::zero(), addr2, &v, expect_id_addr2);

    // fail to send to non existent id actor (vm doesn't create those on send)
    let mres = v
        .apply_message(
            addr1,
            Address::new_id(88),
            TokenAmount::from_atto(1u8),
            METHOD_SEND,
            RawBytes::default(),
        )
        .unwrap();
    assert_eq!(ExitCode::SYS_INVALID_RECEIVER, mres.code);
    assert_account_actor(3, TokenAmount::from_atto(42u8), addr1, &v, expect_id_addr1);
    assert_account_actor(2, TokenAmount::zero(), addr2, &v, expect_id_addr2);
    v.assert_state_invariants();
}

fn assert_embryo_actor(exp_bal: TokenAmount, v: &VM, addr: Address) {
    let act = v.get_actor(addr).unwrap();
    assert_eq!(EMPTY_ARR_CID, act.head);
    assert_eq!(*EMBRYO_ACTOR_CODE_ID, act.code);
    assert_eq!(exp_bal, act.balance);
}

#[test]
fn test_sent_f4() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    // send to an f4 actor where the address manager exists.
    let addr1 = Address::new_delegated(0, &[]).unwrap();
    assert!(v
        .apply_message(
            TEST_FAUCET_ADDR,
            addr1,
            TokenAmount::from_atto(42u8),
            METHOD_SEND,
            RawBytes::default(),
        )
        .unwrap()
        .code
        .is_success());
    let expect_id_addr1 = Address::new_id(FIRST_TEST_USER_ADDR);
    assert_embryo_actor(TokenAmount::from_atto(42u8), &v, expect_id_addr1);

    // Send to an f4 actor where the previous send created the "manager". Importantly, we don't have to deploy code first.
    let addr2 = Address::new_delegated(FIRST_TEST_USER_ADDR, &[]).unwrap();
    assert!(v
        .apply_message(
            TEST_FAUCET_ADDR,
            addr2,
            TokenAmount::from_atto(2u8),
            METHOD_SEND,
            RawBytes::default(),
        )
        .unwrap()
        .code
        .is_success());
    let expect_id_addr2 = Address::new_id(FIRST_TEST_USER_ADDR + 1);
    assert_embryo_actor(TokenAmount::from_atto(2u8), &v, expect_id_addr2);

    // Fail to send to to an f4 actor where the "manager" doesn't exist.
    let addr2 = Address::new_delegated(FIRST_TEST_USER_ADDR + 2, &[]).unwrap();
    assert_eq!(
        v.apply_message(
            TEST_FAUCET_ADDR,
            addr2,
            TokenAmount::from_atto(2u8),
            METHOD_SEND,
            RawBytes::default(),
        )
        .unwrap()
        .code,
        fvm_shared::error::ExitCode::SYS_INVALID_RECEIVER
    );
}

#[test]
fn test_pk_gen() {
    let addrs = pk_addrs_from(5, 2);
    let second_addr_seeded_five = addrs[1];
    let addrs = pk_addrs_from(6, 1);
    let first_addr_seeded_six = addrs[0];
    assert_ne!(second_addr_seeded_five, first_addr_seeded_six);
}
