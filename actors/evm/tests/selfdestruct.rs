use fil_actor_evm::{
    EvmContractActor, Method, ResurrectParams, State, Tombstone, EVM_CONTRACT_SELFDESTRUCT_FAILED,
};
use fil_actors_evm_shared::{address::EthAddress, uints::U256};
use fil_actors_runtime::{test_utils::*, EAM_ACTOR_ADDR, INIT_ACTOR_ADDR};
use fvm_ipld_encoding::{ipld_block::IpldBlock, BytesSer, RawBytes};
use fvm_shared::{
    address::Address,
    econ::TokenAmount,
    error::{ErrorNumber, ExitCode},
    sys::SendFlags,
    MethodNum, METHOD_SEND,
};
use num_traits::Zero;

mod util;

#[test]
fn test_selfdestruct() {
    let bytecode = hex::decode(include_str!("contracts/selfdestruct.hex")).unwrap();

    let contract = Address::new_id(100);
    let beneficiary_id = 1001;
    let beneficiary = Address::new_id(beneficiary_id);

    let token_amount = TokenAmount::from_whole(2);

    let rt = util::init_construct_and_verify(bytecode.clone(), |rt| {
        rt.actor_code_cids.borrow_mut().insert(contract, *EVM_ACTOR_CODE_ID);
        rt.set_origin(contract);
        rt.set_balance(token_amount.clone());
    });

    let returnone_params = hex::decode("901717d1").unwrap();
    let selfdestruct_params = hex::decode("35f46994").unwrap();
    let resurrect_params = IpldBlock::serialize_cbor(&ResurrectParams {
        creator: EthAddress([0; 20]),
        initcode: RawBytes::new(bytecode),
    })
    .unwrap();

    rt.expect_send_simple(beneficiary, METHOD_SEND, None, token_amount, None, ExitCode::OK);

    assert!(util::invoke_contract(&rt, &selfdestruct_params).is_empty());
    let state: State = rt.get_state();
    assert_eq!(state.tombstone, Some(Tombstone { origin: 100, nonce: 0 }));
    rt.verify();

    // Calls still work.
    assert_eq!(U256::from_big_endian(&util::invoke_contract(&rt, &returnone_params)), U256::ONE);
    rt.verify();

    // Try to resurrect during the same "epoch". This should be forbidden.
    rt.set_caller(*EAM_ACTOR_CODE_ID, EAM_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![EAM_ACTOR_ADDR]);
    assert_eq!(
        rt.call::<EvmContractActor>(Method::Resurrect as MethodNum, resurrect_params.clone())
            .unwrap_err()
            .exit_code(),
        ExitCode::USR_FORBIDDEN
    );
    rt.verify();

    rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR); // doesn't really matter

    // Selfdestruct should be callable multiple times, and it shouldn't do anything (but move
    // remaining funds, again).
    rt.expect_validate_caller_any();
    rt.expect_send_simple(beneficiary, METHOD_SEND, None, TokenAmount::zero(), None, ExitCode::OK);
    assert!(util::invoke_contract(&rt, &selfdestruct_params).is_empty());
    rt.verify();

    // Ok, call from a different origin so that the tombstone prevents any calls.
    rt.set_origin(beneficiary);

    // All calls should now do nothing (but still work).
    assert!(util::invoke_contract(&rt, &returnone_params).is_empty());
    rt.verify();

    // We should now be able to resurrect.
    rt.set_caller(*EAM_ACTOR_CODE_ID, EAM_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![EAM_ACTOR_ADDR]);
    rt.call::<EvmContractActor>(Method::Resurrect as MethodNum, resurrect_params).unwrap();
    rt.verify();

    // The tombstone should be gone!
    let state: State = rt.get_state();
    assert_eq!(state.tombstone, None);

    rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR); // doesn't really matter

    // And calls should work again.
    assert_eq!(U256::from_big_endian(&util::invoke_contract(&rt, &returnone_params)), U256::ONE);
    rt.verify();
}

#[test]
fn test_selfdestruct_missing_beneficiary() {
    let bytecode = hex::decode(include_str!("contracts/selfdestruct.hex")).unwrap();

    let contract = Address::new_id(100);
    let beneficiary = Address::new_id(1001);

    let rt = util::init_construct_and_verify(bytecode, |rt| {
        rt.actor_code_cids.borrow_mut().insert(contract, *EVM_ACTOR_CODE_ID);
        rt.set_origin(contract);
    });

    let solidity_params = hex::decode("35f46994").unwrap();
    rt.expect_validate_caller_any();
    rt.expect_send(
        beneficiary,
        METHOD_SEND,
        None,
        rt.get_balance(),
        None,
        SendFlags::default(),
        None,
        ExitCode::OK, // doesn't matter
        Some(ErrorNumber::NotFound),
    );

    // It still works even if the beneficiary doesn't exist.

    assert_eq!(
        rt.call::<EvmContractActor>(
            Method::InvokeContract as u64,
            IpldBlock::serialize_cbor(&BytesSer(&solidity_params)).unwrap(),
        )
        .expect_err("call should have failed")
        .exit_code(),
        EVM_CONTRACT_SELFDESTRUCT_FAILED,
    );
    let state: State = rt.get_state();
    assert_eq!(state.tombstone, None);
    rt.verify();
}
