use fil_actor_evm::{
    interpreter::{address::EthAddress, U256},
    EvmContractActor, Method, ResurrectParams, State, Tombstone,
};
use fil_actors_runtime::{test_utils::*, EAM_ACTOR_ADDR, INIT_ACTOR_ADDR};
use fvm_ipld_encoding::{ipld_block::IpldBlock, RawBytes};
use fvm_shared::{address::Address, econ::TokenAmount, error::ExitCode, MethodNum, METHOD_SEND};
use num_traits::Zero;

mod util;

#[test]
fn test_selfdestruct() {
    let bytecode = hex::decode(include_str!("contracts/selfdestruct.hex")).unwrap();

    let contract = Address::new_id(100);
    let beneficiary_id = 1001;
    let beneficiary = Address::new_id(beneficiary_id);

    let token_amount = TokenAmount::from_whole(2);

    let mut rt = util::init_construct_and_verify(bytecode.clone(), |rt| {
        rt.actor_code_cids.insert(contract, *EVM_ACTOR_CODE_ID);
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

    rt.expect_send(beneficiary, METHOD_SEND, None, token_amount, None, ExitCode::OK);

    assert!(util::invoke_contract(&mut rt, &selfdestruct_params).is_empty());
    let state: State = rt.get_state();
    assert_eq!(state.tombstone, Some(Tombstone { origin: 100, nonce: 0 }));
    rt.verify();

    // Calls still work.
    assert_eq!(
        U256::from_big_endian(&util::invoke_contract(&mut rt, &returnone_params)),
        U256::ONE
    );
    rt.verify();

    // Try to resurrect during the same "epoch". This should be forbidden.
    rt.set_caller(*EAM_ACTOR_CODE_ID, EAM_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![EAM_ACTOR_ADDR]);
    assert_eq!(
        rt.call::<EvmContractActor>(Method::Resurrect as MethodNum, resurrect_params.clone(),)
            .unwrap_err()
            .exit_code(),
        ExitCode::USR_FORBIDDEN
    );
    rt.verify();

    rt.set_caller(*INIT_ACTOR_CODE_ID, INIT_ACTOR_ADDR); // doesn't really matter

    // Selfdestruct should be callable multiple times, and it shouldn't do anything (but move
    // remaining funds, again).
    rt.expect_validate_caller_any();
    rt.expect_send(beneficiary, METHOD_SEND, None, TokenAmount::zero(), None, ExitCode::OK);
    assert!(util::invoke_contract(&mut rt, &selfdestruct_params).is_empty());
    rt.verify();

    // Ok, call from a different origin so that the tombstone prevents any calls.
    rt.set_origin(beneficiary);

    // All calls should now do nothing (but still work).
    assert!(util::invoke_contract(&mut rt, &returnone_params).is_empty());
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
    assert_eq!(
        U256::from_big_endian(&util::invoke_contract(&mut rt, &returnone_params)),
        U256::ONE
    );
    rt.verify();
}

#[test]
fn test_selfdestruct_missing_beneficiary() {
    let bytecode = hex::decode(include_str!("contracts/selfdestruct.hex")).unwrap();

    let contract = Address::new_id(100);
    let beneficiary = Address::new_id(1001);

    let mut rt = util::init_construct_and_verify(bytecode, |rt| {
        rt.actor_code_cids.insert(contract, *EVM_ACTOR_CODE_ID);
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
        ExitCode::SYS_INVALID_RECEIVER,
    );

    // It still works even if the beneficiary doesn't exist.
    assert!(util::invoke_contract(&mut rt, &solidity_params).is_empty());
    let state: State = rt.get_state();
    assert_eq!(state.tombstone, Some(Tombstone { origin: 100, nonce: 0 }));
    rt.verify();
}

#[test]
fn test_selfdestruct_during_constructor() {
    let bytecode = hex::decode(include_str!("contracts/AutoSelfDestruct.hex")).unwrap();

    let contract = Address::new_id(100);

    let mut rt = util::init_construct_and_verify(bytecode, |rt| {
        rt.set_balance(TokenAmount::from_whole(1));

        rt.actor_code_cids.insert(contract, *EVM_ACTOR_CODE_ID);
        rt.set_origin(contract);

        // test util uses EAM address for constructor
        rt.expect_send(
            Address::new_id(10),
            METHOD_SEND,
            None,
            rt.get_balance(),
            None,
            ExitCode::OK,
        );
    });

    let solidity_params = hex::decode("83197ef0").unwrap();
    rt.expect_validate_caller_any();

    assert!(util::invoke_contract(&mut rt, &solidity_params).is_empty());
    let state: State = rt.get_state();
    assert_eq!(state.tombstone, Some(Tombstone { origin: 100, nonce: 0 }));
    rt.verify();
}
