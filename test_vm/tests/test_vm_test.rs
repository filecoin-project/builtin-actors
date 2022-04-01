use test_vm::{
    VM, Actor, actor,
};
use cid::{multihash, Cid};
use fvm_shared::blockstore::{MemoryBlockstore};
use fil_actors_runtime::{
    INIT_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
};
use fil_actors_runtime::test_utils::{ACCOUNT_ACTOR_CODE_ID, PAYCH_ACTOR_CODE_ID, make_builtin};
use fvm_shared::econ::TokenAmount;
use fvm_shared::address::{Address};


#[test]
fn state_control() {
    let mut store = MemoryBlockstore::new();
    let mut v = VM::new(&store);

    // set actor
    let a1 = actor(
        *ACCOUNT_ACTOR_CODE_ID,
        make_builtin(b"a1-head"),
        42,
        TokenAmount::from(10u8),
    );
    v.set_actor(Address::new_id(1000), a1.clone()).unwrap();
    let out = v.get_actor(Address::new_id(1000)).unwrap();
    assert_eq!(out, a1);
    let check = v.checkpoint().unwrap();

    let a2 = actor(*PAYCH_ACTOR_CODE_ID, make_builtin(b"a2-head"), 88, TokenAmount::from(1u8));
    v.set_actor(Address::new_id(2222), a2.clone()).unwrap();
    assert_eq!(v.get_actor(Address::new_id(2222)).unwrap(), a2);
    // rollback removes a2 but not a1
    v.rollback(check).unwrap();

    // a2 is gone
    v.get_actor(Address::new_id(2222)).expect_err("a2 should be rolled back");
    assert_eq!(v.get_actor(Address::new_id(1000)).unwrap(), a1);

}

//#[test]
//fn normalize_addr() {
    // set up init actor

    // set up account actor

    // set up system actor

    // normalize system actor => nop

    // normalize account actor => nop

    // normalize account actor by id addr => resolves 
//}
