use fil_actors_runtime::{test_utils::{make_builtin, ACCOUNT_ACTOR_CODE_ID, PAYCH_ACTOR_CODE_ID}, INIT_ACTOR_ADDR};
use fil_actor_account::{State as AccountState};
use fil_actors_runtime::{FIRST_NON_SINGLETON_ADDR};
use fvm_shared::{METHOD_SEND};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_ipld_encoding::{RawBytes};
use fvm_shared::error::ExitCode;
use test_vm::{actor, VM};

#[test]
fn state_control() {
    let store = MemoryBlockstore::new();
    let v = VM::new(&store);
    let addr1 = Address::new_id(1000);
    let addr2 = Address::new_id(2222);

    // set actor
    let a1 = actor(*ACCOUNT_ACTOR_CODE_ID, make_builtin(b"a1-head"), 42, TokenAmount::from(10u8));
    v.set_actor(addr1, a1.clone());
    let out = v.get_actor(addr1).unwrap();
    assert_eq!(out, a1);
    let check = v.checkpoint();

    let a2 = actor(*PAYCH_ACTOR_CODE_ID, make_builtin(b"a2-head"), 88, TokenAmount::from(1u8));
    v.set_actor(addr2, a2.clone());
    assert_eq!(v.get_actor(addr2).unwrap(), a2);
    // rollback removes a2 but not a1
    v.rollback(check);

    // a2 is gone
    assert_eq!(None, v.get_actor(addr2));
    assert_eq!(v.get_actor(addr1).unwrap(), a1);
}

fn assert_account_actor(exp_call_seq: u64, exp_bal: TokenAmount, exp_pk_addr: Address, v: &VM, addr: Address) {
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
    v.apply_message(*INIT_ACTOR_ADDR, addr1, TokenAmount::from(42u8),  METHOD_SEND, RawBytes::default()).unwrap();
    let expect_id_addr1 = Address::new_id(FIRST_NON_SINGLETON_ADDR);
    assert_account_actor(0, TokenAmount::from(42u8), addr1, &v, expect_id_addr1);

    // send from this account actor to another uninit account actor
    let addr2 = Address::new_bls(&[2; fvm_shared::address::BLS_PUB_LEN]).unwrap();
    v.apply_message(addr1, addr2, TokenAmount::from(41u8), METHOD_SEND, RawBytes::default()).unwrap();
    let expect_id_addr2 = Address::new_id(FIRST_NON_SINGLETON_ADDR + 1);
    assert_account_actor(0, TokenAmount::from(41u8), addr2,  &v, expect_id_addr2);

    // send between two initialized account actors
    v.apply_message(addr2, addr1, TokenAmount::from(41u8), METHOD_SEND, RawBytes::default()).unwrap();
    assert_account_actor(1, TokenAmount::from(42u8), addr1, &v, expect_id_addr1);
    assert_account_actor(1, TokenAmount::from(0u8), addr2, &v, expect_id_addr2);

    // fail with insufficient funds
    let mres = v.apply_message(addr2, addr1, TokenAmount::from(1u8), METHOD_SEND, RawBytes::default()).unwrap();
    assert_eq!(ExitCode::SYS_INSUFFICIENT_FUNDS, mres.code);
    assert_account_actor(1, TokenAmount::from(42u8), addr1, &v, expect_id_addr1);
    assert_account_actor(2, TokenAmount::from(0u8), addr2, &v, expect_id_addr2);

    // fail to send to non existent id actor (vm doesn't create those on send)
    let mres = v.apply_message(addr1, Address::new_id(99), TokenAmount::from(1u8), METHOD_SEND, RawBytes::default()).unwrap();
    assert_eq!(ExitCode::SYS_INVALID_RECEIVER, mres.code);
    assert_account_actor(2, TokenAmount::from(42u8), addr1, &v, expect_id_addr1);
    assert_account_actor(2, TokenAmount::from(0u8), addr2, &v, expect_id_addr2);
}