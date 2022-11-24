use fil_actor_init::Exec4Return;
use fil_actors_runtime::{
    runtime::EMPTY_ARR_CID,
    test_utils::{EAM_ACTOR_CODE_ID, EMBRYO_ACTOR_CODE_ID, MULTISIG_ACTOR_CODE_ID},
    EAM_ACTOR_ADDR, EAM_ACTOR_ID, INIT_ACTOR_ADDR,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::ipld_block::IpldBlock;
use fvm_shared::{address::Address, econ::TokenAmount, error::ExitCode, METHOD_SEND};
use num_traits::Zero;
use test_vm::{actor, FIRST_TEST_USER_ADDR, TEST_FAUCET_ADDR, VM};

fn assert_embryo_actor(exp_bal: TokenAmount, v: &VM, addr: Address) {
    let act = v.get_actor(addr).unwrap();
    assert_eq!(EMPTY_ARR_CID, act.head);
    assert_eq!(*EMBRYO_ACTOR_CODE_ID, act.code);
    assert_eq!(exp_bal, act.balance);
}

#[test]
fn embryo_deploy() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    // Create a "fake" eam.
    v.set_actor(
        EAM_ACTOR_ADDR,
        actor(*EAM_ACTOR_CODE_ID, EMPTY_ARR_CID, 0, TokenAmount::zero(), None),
    );

    // Create an embryo.

    let subaddr = b"foobar";
    let addr = Address::new_delegated(EAM_ACTOR_ID, subaddr).unwrap();
    assert!(v
        .apply_message(
            TEST_FAUCET_ADDR,
            addr,
            TokenAmount::from_atto(42u8),
            METHOD_SEND,
            RawBytes::default(),
        )
        .unwrap()
        .code
        .is_success());
    let expect_id_addr = Address::new_id(FIRST_TEST_USER_ADDR);
    assert_embryo_actor(TokenAmount::from_atto(42u8), &v, expect_id_addr);

    // Make sure we assigned the right f4 address.
    assert_eq!(v.normalize_address(&addr).unwrap(), expect_id_addr);

    // Deploy a multisig to the embryo.
    let msig_ctor_params = Some(
        IpldBlock::serialize_cbor(&fil_actor_multisig::ConstructorParams {
            signers: vec![EAM_ACTOR_ADDR],
            num_approvals_threshold: 1,
            unlock_duration: 0,
            start_epoch: 0,
        })
        .unwrap(),
    );

    let deploy = || {
        v.apply_message(
            EAM_ACTOR_ADDR, // so this works even if "m2-native" is disabled.
            INIT_ACTOR_ADDR,
            TokenAmount::zero(),
            fil_actor_init::Method::Exec4 as u64,
            fil_actor_init::Exec4Params {
                code_cid: *MULTISIG_ACTOR_CODE_ID,
                constructor_params: msig_ctor_params.clone(),
                subaddress: subaddr[..].to_owned().into(),
            },
        )
        .unwrap()
    };

    let msig_ctor_res = deploy();
    assert_eq!(msig_ctor_res.code, ExitCode::OK);
    let msig_ctor_ret: Exec4Return = msig_ctor_res.ret.deserialize().unwrap();

    assert_eq!(
        expect_id_addr, msig_ctor_ret.id_address,
        "expected actor to be deployed over embryo"
    );

    // Make sure we kept the balance.
    assert_eq!(v.get_actor(expect_id_addr).unwrap().balance, TokenAmount::from_atto(42u8));

    // Try to overwrite it.
    let msig_ctor_res = deploy();
    assert_eq!(ExitCode::USR_FORBIDDEN, msig_ctor_res.code);
}
