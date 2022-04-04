use fil_actors_runtime::test_utils::{make_builtin, ACCOUNT_ACTOR_CODE_ID, PAYCH_ACTOR_CODE_ID};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use test_vm::{actor, VM};

#[test]
fn state_control() {
    let store = MemoryBlockstore::new();
    let mut v = VM::new(&store);
    let addr1 = Address::new_id(1000);
    let addr2 = Address::new_id(2222);

    // set actor
    let a1 = actor(*ACCOUNT_ACTOR_CODE_ID, make_builtin(b"a1-head"), 42, TokenAmount::from(10u8));
    v.set_actor(&addr1, a1.clone());
    let out = v.get_actor(&addr1).unwrap();
    assert_eq!(out, &a1);
    let check = v.checkpoint();

    let a2 = actor(*PAYCH_ACTOR_CODE_ID, make_builtin(b"a2-head"), 88, TokenAmount::from(1u8));
    v.set_actor(&addr2, a2.clone());
    assert_eq!(v.get_actor(&addr2).unwrap(), &a2);
    // rollback removes a2 but not a1
    v.rollback(&check);

    // a2 is gone
    assert_eq!(None, v.get_actor(&addr2));
    assert_eq!(v.get_actor(&addr1).unwrap(), &a1);
}
