use alloy_core::sol_types::{SolCall, SolInterface, decode_revert_reason};
use alloy_core::{primitives::Address as EthAddress, sol};

use export_macro::vm_test;
use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::{
    EAM_ACTOR_ADDR, EAM_ACTOR_ID, test_utils::ETHACCOUNT_ACTOR_CODE_ID,
    test_utils::EVM_ACTOR_CODE_ID,
};
use fvm_ipld_encoding::RawBytes;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{BytesDe, strict_bytes};
use fvm_shared::ActorID;
use fvm_shared::METHOD_SEND;
use fvm_shared::{address::Address, econ::TokenAmount};
use num_traits::Zero;
use serde::{Deserialize, Serialize};
use vm_api::VM;
use vm_api::util::{apply_ok, serialize_ok};

use crate::TEST_FAUCET_ADDR;
use crate::util::create_accounts;

// Generate a statically typed interface for the contracts.
sol!("../actors/evm/tests/contracts/Recursive.sol");
sol!("../actors/evm/tests/contracts/Lifecycle.sol");
sol!("../actors/evm/tests/contracts/TransientStorageTest.sol");

pub fn id_to_eth(id: ActorID) -> EthAddress {
    let mut addr = [0u8; 20];
    addr[0] = 0xff;
    addr[12..].copy_from_slice(&id.to_be_bytes());
    EthAddress::from_slice(&addr)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContractParams(#[serde(with = "strict_bytes")] pub Vec<u8>);

#[vm_test]
pub fn evm_eth_create_external_test(v: &dyn VM) {
    // create the EthAccount
    let eth_bits = hex_literal::hex!("FEEDFACECAFEBEEF000000000000000000000000");
    let eth_addr = Address::new_delegated(EAM_ACTOR_ID, &eth_bits).unwrap();
    apply_ok(
        v,
        &TEST_FAUCET_ADDR,
        &eth_addr,
        &TokenAmount::from_whole(10_000),
        METHOD_SEND,
        None::<RawBytes>,
    );

    let account = v.resolve_id_address(&eth_addr).unwrap();

    let mut actor = v.actor(&account).unwrap();
    actor.code = *ETHACCOUNT_ACTOR_CODE_ID;
    v.set_actor(&account, actor);

    // now create an empty contract
    let params = IpldBlock::serialize_cbor(&fil_actor_eam::CreateExternalParams(vec![])).unwrap();
    let create_result = v
        .execute_message(
            &account,
            &EAM_ACTOR_ADDR,
            &TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            params,
        )
        .unwrap();

    assert!(
        create_result.code.is_success(),
        "failed to create the new actor {}",
        create_result.message
    );

    // and call it
    let create_return: fil_actor_eam::CreateExternalReturn =
        create_result.ret.unwrap().deserialize().expect("failed to decode results");

    let robust_addr = create_return.robust_address.unwrap();

    let params = IpldBlock::serialize_cbor(&ContractParams(vec![])).unwrap();
    let call_result = v
        .execute_message(
            &account,
            &robust_addr,
            &TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            params,
        )
        .unwrap();
    assert!(call_result.code.is_success(), "failed to call the new actor {}", call_result.message);
}

#[vm_test]
pub fn evm_call_test(v: &dyn VM) {
    let account = create_accounts(v, 1, &TokenAmount::from_whole(10_000))[0];

    let bytecode =
        hex::decode(include_str!("../../../actors/evm/tests/contracts/Recursive.hex")).unwrap();

    let create_result = v
        .execute_message(
            &account,
            &EAM_ACTOR_ADDR,
            &TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            Some(serialize_ok(&fil_actor_eam::CreateExternalParams(bytecode))),
        )
        .unwrap();

    assert!(
        create_result.code.is_success(),
        "failed to create the new actor {}",
        create_result.message
    );

    let create_return: fil_actor_eam::CreateExternalReturn =
        create_result.ret.unwrap().deserialize().expect("failed to decode results");

    let contract_params = Recursive::enterCall::new(()).abi_encode();
    let call_result = v
        .execute_message(
            &account,
            &create_return.robust_address.unwrap(),
            &TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            Some(serialize_ok(&ContractParams(contract_params.to_vec()))),
        )
        .unwrap();
    assert!(call_result.code.is_success(), "failed to call the new actor {}", call_result.message);

    let BytesDe(return_value) =
        call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
    let evm_ret =
        Recursive::enterCall::abi_decode_returns(&return_value).expect("failed to decode return");
    assert_eq!(0, evm_ret, "expected contract to return 0 on success");
}

#[vm_test]
pub fn evm_create_test(v: &dyn VM) {
    let account = create_accounts(v, 1, &TokenAmount::from_whole(10_000))[0];

    let bytecode =
        hex::decode(include_str!("../../../actors/evm/tests/contracts/Lifecycle.hex")).unwrap();

    let create_result = v
        .execute_message(
            &account,
            &EAM_ACTOR_ADDR,
            &TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            Some(serialize_ok(&fil_actor_eam::CreateExternalParams(bytecode))),
        )
        .unwrap();

    assert!(
        create_result.code.is_success(),
        "failed to create the new actor {}",
        create_result.message
    );

    let create_return: fil_actor_eam::CreateExternalReturn =
        create_result.ret.unwrap().deserialize().expect("failed to decode results");

    let test_func = |create_func: Factory::FactoryCalls, recursive: bool| {
        let child_addr_eth = {
            let call_params = create_func.abi_encode();
            let call_result = v
                .execute_message(
                    &account,
                    &create_return.robust_address.unwrap(),
                    &TokenAmount::zero(),
                    fil_actor_evm::Method::InvokeContract as u64,
                    Some(serialize_ok(&ContractParams(call_params.to_vec()))),
                )
                .unwrap();
            assert!(
                call_result.code.is_success(),
                "failed to call the new actor {}",
                call_result.message,
            );
            let BytesDe(return_value) =
                call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
            Factory::createCall::abi_decode_returns(&return_value).expect("failed to decode return")
        };

        let child_addr = Address::new_delegated(EAM_ACTOR_ID, &child_addr_eth.0[..]).unwrap();

        // Verify the child.
        {
            let call_params = FactoryChild::get_valueCall::new(()).abi_encode();
            let call_result = v
                .execute_message(
                    &account,
                    &child_addr,
                    &TokenAmount::zero(),
                    fil_actor_evm::Method::InvokeContract as u64,
                    Some(serialize_ok(&ContractParams(call_params.to_vec()))),
                )
                .unwrap();
            assert!(
                call_result.code.is_success(),
                "failed to call the new actor {}",
                call_result.message
            );
            let BytesDe(return_value) =
                call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
            let res = FactoryChild::get_valueCall::abi_decode_returns(&return_value)
                .expect("failed to decode return");
            assert_eq!(res, 42);
        }

        // Kill it.
        {
            let call_params = if recursive {
                FactoryChild::FactoryChildCalls::dieRecursive(().into())
            } else {
                FactoryChild::FactoryChildCalls::die(().into())
            }
            .abi_encode();
            let call_result = v
                .execute_message(
                    &account,
                    &child_addr,
                    &TokenAmount::zero(),
                    fil_actor_evm::Method::InvokeContract as u64,
                    Some(serialize_ok(&ContractParams(call_params))),
                )
                .unwrap();
            assert!(
                call_result.code.is_success(),
                "failed to call the new actor {}",
                call_result.message
            );
        }

        // It should now be dead.
        {
            let call_params = FactoryChild::get_valueCall::new(()).abi_encode();
            let call_result = v
                .execute_message(
                    &account,
                    &child_addr,
                    &TokenAmount::zero(),
                    fil_actor_evm::Method::InvokeContract as u64,
                    Some(serialize_ok(&ContractParams(call_params.to_vec()))),
                )
                .unwrap();
            assert!(
                call_result.code.is_success(),
                "failed to call the new actor {}",
                call_result.message
            );
            let BytesDe(return_value) =
                call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
            assert!(return_value.is_empty());
        }
        child_addr_eth
    };

    // Test CREATE2 twice because we should be able to deploy over an existing contract.
    let eth_addr1 = test_func(Factory::FactoryCalls::create2(([0; 32].into(), 42).into()), false);
    let eth_addr2 = test_func(Factory::FactoryCalls::create2(([0; 32].into(), 42).into()), false);
    assert_eq!(eth_addr1, eth_addr2);

    // Recursive self-destruct should work.
    let eth_addr1 = test_func(Factory::FactoryCalls::create2(([1; 32].into(), 42).into()), true);
    let eth_addr2 = test_func(Factory::FactoryCalls::create2(([1; 32].into(), 42).into()), false);
    assert_eq!(eth_addr1, eth_addr2);

    // Then test create and expect two different addrs.
    let eth_addr1 = test_func(Factory::FactoryCalls::create((42,).into()), false);
    let eth_addr2 = test_func(Factory::FactoryCalls::create((42,).into()), false);
    assert_ne!(eth_addr1, eth_addr2);

    // Then test a failure

    {
        let call_params = Factory::createCall::new((-1,)).abi_encode();
        let call_result = v
            .execute_message(
                &account,
                &create_return.robust_address.unwrap(),
                &TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(serialize_ok(&ContractParams(call_params.to_vec()))),
            )
            .unwrap();
        assert_eq!(
            call_result.code.value(),
            33,
            "expected contract revert {}",
            call_result.message
        );
        let BytesDe(return_value) =
            call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
        let revert_msg: String = decode_revert_reason(&return_value).expect("expected a revert");
        assert_eq!(revert_msg, "revert: create failed");
    }
}

#[vm_test]
pub fn evm_empty_initcode_test(v: &dyn VM) {
    let account = create_accounts(v, 1, &TokenAmount::from_whole(10_000))[0];
    let create_result = v
        .execute_message(
            &account,
            &EAM_ACTOR_ADDR,
            &TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            Some(serialize_ok(&fil_actor_eam::CreateExternalParams(vec![]))),
        )
        .unwrap();

    assert!(
        create_result.code.is_success(),
        "failed to create the new actor {}",
        create_result.message
    );
}

#[vm_test]
#[allow(non_snake_case)]
pub fn evm_staticcall_test(v: &dyn VM) {
    // test scenarios:
    // one hop:
    // A -> staticcall -> B (read) OK
    // A -> staticcall -> B (write) FAIL
    // two hop sticky:
    // A -> staticcall -> B -> call -> C (read) OK
    // A -> staticcall -> B -> call -> C (write) FAIL
    let accounts = create_accounts(v, 3, &TokenAmount::from_whole(10_000));

    let bytecode =
        hex::decode(include_str!("../../../actors/evm/tests/contracts/callvariants.hex")).unwrap();

    let created: Vec<_> = accounts
        .iter()
        .map(|account| {
            let create_result = v
                .execute_message(
                    account,
                    &EAM_ACTOR_ADDR,
                    &TokenAmount::zero(),
                    fil_actor_eam::Method::CreateExternal as u64,
                    Some(serialize_ok(&fil_actor_eam::CreateExternalParams(bytecode.clone()))),
                )
                .unwrap();

            assert!(
                create_result.code.is_success(),
                "failed to create the new actor {}",
                create_result.message
            );

            let create_return: fil_actor_eam::CreateExternalReturn =
                create_result.ret.unwrap().deserialize().expect("failed to decode results");

            // Make sure we deployed an EVM actor.
            assert_eq!(
                &v.actor(&Address::new_id(create_return.actor_id)).unwrap().code,
                &*EVM_ACTOR_CODE_ID,
            );

            create_return
        })
        .collect();

    // A -> staticcall -> B (read) OK
    {
        let A_act = accounts[0];
        let A_robust_addr = created[0].robust_address.unwrap();
        let B = id_to_eth(created[1].actor_id);
        let mut params = [0u8; 36];
        params[3] = 1;
        params[16..].copy_from_slice(B.as_ref());

        let call_result = v
            .execute_message(
                &A_act,
                &A_robust_addr,
                &TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(serialize_ok(&ContractParams(params.to_vec()))),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.message
        );
        let BytesDe(return_value) =
            call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
        assert_eq!(&return_value[12..], &created[1].eth_address.0);
    }

    // A -> staticcall -> B (write) FAIL
    {
        let A_act = accounts[0];
        let A_robust_addr = created[0].robust_address.unwrap();
        let B = id_to_eth(created[1].actor_id);
        let mut params = [0u8; 36];
        params[3] = 3;
        params[16..].copy_from_slice(B.as_ref());

        let call_result = v
            .execute_message(
                &A_act,
                &A_robust_addr,
                &TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(serialize_ok(&ContractParams(params.to_vec()))),
            )
            .unwrap();
        assert_eq!(call_result.code.value(), 33, "static call mutation did not revert");
    }

    // A -> staticcall -> B -> call -> C (read) OK
    {
        let A_act = accounts[0];
        let A_robust_addr = created[0].robust_address.unwrap();
        let B = id_to_eth(created[1].actor_id);
        let C = id_to_eth(created[2].actor_id);
        let mut params = [0u8; 68];
        params[3] = 5;
        params[16..][..20].copy_from_slice(B.as_ref());
        params[48..].copy_from_slice(C.as_ref());

        let call_result = v
            .execute_message(
                &A_act,
                &A_robust_addr,
                &TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(serialize_ok(&ContractParams(params.to_vec()))),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.message
        );
        let BytesDe(return_value) =
            call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
        assert_eq!(&return_value[12..], &created[2].eth_address.0);
    }

    // A -> staticcall -> B -> call -> C (write) FAIL
    {
        let A_act = accounts[0];
        let A_robust_addr = created[0].robust_address.unwrap();
        let B = id_to_eth(created[1].actor_id);
        let C = id_to_eth(created[2].actor_id);
        let mut params = [0u8; 68];
        params[3] = 7;
        params[16..][..20].copy_from_slice(B.as_ref());
        params[48..].copy_from_slice(C.as_ref());

        let call_result = v
            .execute_message(
                &A_act,
                &A_robust_addr,
                &TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(serialize_ok(&ContractParams(params.to_vec()))),
            )
            .unwrap();
        assert_eq!(call_result.code.value(), 33, "static call mutation did not revert");
    }
}

#[allow(non_snake_case)]
#[vm_test]
pub fn evm_delegatecall_test(v: &dyn VM) {
    // test scenarios:
    // one hop:
    // A -> delegatecall -> B (read) OK
    // A -> delegatecall -> B (write) -> return (read) OK
    // two hop with sticky staticcall:
    // A -> staticcall -> B -> delegatecall -> C (read) OK
    // A -> staticcall -> B -> delegatecall -> C (write) FAIL
    let accounts = create_accounts(v, 3, &TokenAmount::from_whole(10_000));

    let bytecode =
        hex::decode(include_str!("../../../actors/evm/tests/contracts/callvariants.hex")).unwrap();

    let created: Vec<_> = accounts
        .iter()
        .map(|account| {
            let create_result = v
                .execute_message(
                    account,
                    &EAM_ACTOR_ADDR,
                    &TokenAmount::zero(),
                    fil_actor_eam::Method::CreateExternal as u64,
                    Some(serialize_ok(&fil_actor_eam::CreateExternalParams(bytecode.clone()))),
                )
                .unwrap();

            assert!(
                create_result.code.is_success(),
                "failed to create the new actor {}",
                create_result.message
            );

            let create_return: fil_actor_eam::CreateExternalReturn =
                create_result.ret.unwrap().deserialize().expect("failed to decode results");

            // Make sure we deployed an EVM actor.
            assert_eq!(
                &v.actor(&Address::new_id(create_return.actor_id)).unwrap().code,
                &*EVM_ACTOR_CODE_ID
            );

            create_return
        })
        .collect();

    // A -> delegatecall -> B (read) OK
    {
        let A_act = accounts[0];
        let A_robust_addr = created[0].robust_address.unwrap();
        let B = id_to_eth(created[1].actor_id);
        let mut params = [0u8; 36];
        params[3] = 9;
        params[16..].copy_from_slice(B.as_ref());

        let call_result = v
            .execute_message(
                &A_act,
                &A_robust_addr,
                &TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(serialize_ok(&ContractParams(params.to_vec()))),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.message
        );
        let BytesDe(return_value) =
            call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
        assert_eq!(&return_value[12..], &created[0].eth_address.0);
    }

    // A -> delegatecall -> B (write) -> return (read) OK
    {
        let A_act = accounts[0];
        let A_robust_addr = created[0].robust_address.unwrap();
        let B = id_to_eth(created[1].actor_id);
        let mut params = [0u8; 36];
        params[3] = 10;
        params[16..].copy_from_slice(B.as_ref());

        let call_result = v
            .execute_message(
                &A_act,
                &A_robust_addr,
                &TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(serialize_ok(&ContractParams(params.to_vec()))),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.message
        );
        let BytesDe(return_value) =
            call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
        assert_eq!(&return_value[28..], &[0xff, 0xff, 0xff, 0x42]);
    }

    // A -> delegatecall -> B (return value received) OK
    {
        let A_act = accounts[0];
        let A_robust_addr = created[0].robust_address.unwrap();
        let B = id_to_eth(created[1].actor_id);
        let mut params = [0u8; 36];
        params[3] = 16;
        params[16..].copy_from_slice(B.as_ref());

        let value = TokenAmount::from_whole(123);

        let call_result = v
            .execute_message(
                &A_act,
                &A_robust_addr,
                &value,
                fil_actor_evm::Method::InvokeContract as u64,
                Some(serialize_ok(&ContractParams(params.to_vec()))),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.message
        );
        let BytesDe(return_value) =
            call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
        assert_eq!(&return_value, &U256::from(&value).to_bytes()[..]);
    }
}

#[allow(non_snake_case)]
#[vm_test]
pub fn evm_staticcall_delegatecall_test(v: &dyn VM) {
    // test scenarios:
    // one hop:
    // A -> delegatecall -> B (read) OK
    // A -> delegatecall -> B (write) -> return (read) OK
    // two hop with sticky staticcall:
    // A -> staticcall -> B -> delegatecall -> C (read) OK
    // A -> staticcall -> B -> delegatecall -> C (write) FAIL

    let accounts = create_accounts(v, 3, &TokenAmount::from_whole(10_000));

    let bytecode =
        hex::decode(include_str!("../../../actors/evm/tests/contracts/callvariants.hex")).unwrap();

    let created: Vec<_> = accounts
        .iter()
        .map(|account| {
            let create_result = v
                .execute_message(
                    account,
                    &EAM_ACTOR_ADDR,
                    &TokenAmount::zero(),
                    fil_actor_eam::Method::CreateExternal as u64,
                    Some(serialize_ok(&fil_actor_eam::CreateExternalParams(bytecode.clone()))),
                )
                .unwrap();

            assert!(
                create_result.code.is_success(),
                "failed to create the new actor {}",
                create_result.message
            );

            let create_return: fil_actor_eam::CreateExternalReturn =
                create_result.ret.unwrap().deserialize().expect("failed to decode results");

            // Make sure we deployed an EVM actor.
            assert_eq!(
                &v.actor(&Address::new_id(create_return.actor_id)).unwrap().code,
                &*EVM_ACTOR_CODE_ID
            );

            create_return
        })
        .collect();

    // A -> staticcall -> B -> delegatecall -> C (read) OK
    {
        let A_act = accounts[0];
        let A_robust_addr = created[0].robust_address.unwrap();
        let B = id_to_eth(created[1].actor_id);
        let C = id_to_eth(created[2].actor_id);
        let mut params = [0u8; 68];
        params[3] = 11;
        params[16..][..20].copy_from_slice(B.as_ref());
        params[48..].copy_from_slice(C.as_ref());

        let call_result = v
            .execute_message(
                &A_act,
                &A_robust_addr,
                &TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(serialize_ok(&ContractParams(params.to_vec()))),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.message
        );
        let BytesDe(return_value) =
            call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
        assert_eq!(&return_value[12..], &created[1].eth_address.0);
    }

    // A -> staticcall -> B -> delegatecall -> C (write) FAIL
    {
        let A_act = accounts[0];
        let A_robust_addr = created[0].robust_address.unwrap();
        let B = id_to_eth(created[1].actor_id);
        let C = id_to_eth(created[2].actor_id);
        let mut params = [0u8; 68];
        params[3] = 13;
        params[16..][..20].copy_from_slice(B.as_ref());
        params[48..].copy_from_slice(C.as_ref());

        let call_result = v
            .execute_message(
                &A_act,
                &A_robust_addr,
                &TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(serialize_ok(&ContractParams(params.to_vec()))),
            )
            .unwrap();
        assert_eq!(call_result.code.value(), 33, "static call mutation did not revert");
    }
}

#[allow(non_snake_case)]
#[vm_test]
pub fn evm_constructor_delegatecall_regression_test(v: &dyn VM) {
    const IMP_INITCODE: &str =
        include_str!("../../../actors/evm/tests/contracts/recall_contract/implementation.hex");
    const PROXY_INITCODE: &str =
        include_str!("../../../actors/evm/tests/contracts/recall_contract/proxy.hex");

    // Create accounts for deployment
    let deployer = create_accounts(v, 1, &TokenAmount::from_whole(10_000))[0];

    // 1. Deploy the implementation contract
    let imp_bytecode = hex::decode(IMP_INITCODE).unwrap();
    let imp_deploy_result = v
        .execute_message(
            &deployer,
            &EAM_ACTOR_ADDR,
            &TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            Some(serialize_ok(&fil_actor_eam::CreateExternalParams(imp_bytecode))),
        )
        .unwrap();

    assert!(
        imp_deploy_result.code.is_success(),
        "Failed to deploy implementation contract: {}",
        imp_deploy_result.message
    );

    let imp_return: fil_actor_eam::CreateExternalReturn = imp_deploy_result
        .ret
        .unwrap()
        .deserialize()
        .expect("Failed to decode implementation deployment results");

    // Make sure we deployed an EVM actor
    assert_eq!(&v.actor(&Address::new_id(imp_return.actor_id)).unwrap().code, &*EVM_ACTOR_CODE_ID);

    // 2. Deploy the proxy contract with implementation address in constructor, replacing the
    // mainnet implementation address.
    let proxy_initcode_fixed = PROXY_INITCODE.replace(
        "1835374384aa51b169c0705da26a84bb760f2b37",
        &hex::encode(imp_return.eth_address.0),
    );
    let proxy_initcode = hex::decode(proxy_initcode_fixed).unwrap();

    let proxy_deploy_result = v
        .execute_message(
            &deployer,
            &EAM_ACTOR_ADDR,
            &TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            Some(serialize_ok(&fil_actor_eam::CreateExternalParams(proxy_initcode))),
        )
        .unwrap();

    assert!(
        proxy_deploy_result.code.is_success(),
        "Failed to deploy proxy contract: {}",
        proxy_deploy_result.message
    );

    let proxy_return: fil_actor_eam::CreateExternalReturn = proxy_deploy_result
        .ret
        .unwrap()
        .deserialize()
        .expect("Failed to decode proxy deployment results");

    let proxy_robust_addr = proxy_return.robust_address.unwrap();

    // Try to mint:
    const PARAMS: &str = "40c10f1900000000000000000000000090f79bf6eb2c4f870365e785982e1f101e93b9060000000000000000000000000000000000000000000000008ac7230489e80000";
    let params = hex::decode(PARAMS).unwrap();

    let mint_result = v
        .execute_message(
            &deployer,
            &proxy_robust_addr,
            &TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            Some(serialize_ok(&ContractParams(params))),
        )
        .unwrap();

    assert!(mint_result.code.is_success(), "Failed to mint: {}", mint_result.message);
}

#[allow(non_snake_case)]
#[vm_test]
pub fn evm_init_revert_data_test(v: &dyn VM) {
    let account = create_accounts(v, 1, &TokenAmount::from_whole(10_000))[0];
    let create_result = v
        .execute_message(
            &account,
            &EAM_ACTOR_ADDR,
            &TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            // init code:
            // PUSH1 0x42; PUSH1 0x0; MSTORE;
            // PUSH1 0x20; PUSH1 0x0; REVERT
            Some(serialize_ok(&fil_actor_eam::CreateExternalParams(vec![
                0x60, 0x42, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xfd,
            ]))),
        )
        .unwrap();

    assert!(!create_result.code.is_success(), "new actor was successfully created!");

    assert!(create_result.ret.is_some(), "missing return data!");

    let BytesDe(revert_data) =
        create_result.ret.unwrap().deserialize().expect("failed to deserialize revert data");
    let mut expected = [0u8; 32];
    expected[31] = 0x42;
    assert_eq!(revert_data, expected);
}

#[vm_test]
pub fn evm_transient_nested_test(v: &dyn VM) {
    // Step 1: Create an EthAccount with an initial balance.
    let account = create_accounts(v, 1, &TokenAmount::from_whole(10_000))[0];

    // Step 2: Deploy the `TransientStorageTest` contract twice.
    let bytecode =
        hex::decode(include_str!("../../../actors/evm/tests/contracts/TransientStorageTest.hex"))
            .unwrap();

    let create_contract = |account: &Address| {
        let create_result = v
            .execute_message(
                account,
                &EAM_ACTOR_ADDR,
                &TokenAmount::zero(),
                fil_actor_eam::Method::CreateExternal as u64,
                Some(serialize_ok(&fil_actor_eam::CreateExternalParams(bytecode.clone()))),
            )
            .unwrap();

        assert!(
            create_result.code.is_success(),
            "failed to create the contract {}",
            create_result.message
        );

        let create_return: fil_actor_eam::CreateExternalReturn =
            create_result.ret.unwrap().deserialize().expect("failed to decode results");

        create_return.robust_address.unwrap()
    };

    // Deploy two instances of the contract.
    let contract_1_addr = create_contract(&account);
    let contract_2_addr = create_contract(&account);

    // Step 3: Call `testNestedContracts` on the first contract, passing the address of the second.
    let nested_contract_id = v.resolve_id_address(&contract_2_addr).unwrap().id().unwrap();
    let nested_contract_address = id_to_eth(nested_contract_id);

    let call_params =
        TransientStorageTest::testNestedContractsCall::new((nested_contract_address,)).abi_encode();

    let call_result = v
        .execute_message(
            &account,
            &contract_1_addr,
            &TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            Some(serialize_ok(&ContractParams(call_params.to_vec()))),
        )
        .unwrap();

    // Step 4: Check if the call was successful.
    assert!(
        call_result.code.is_success(),
        "testNestedContracts call failed: {}",
        call_result.message
    );

    // Step 5: Deserialize and verify the event emitted by the contract.
    let BytesDe(return_value) =
        call_result.ret.unwrap().deserialize().expect("failed to deserialize results");

    let event_emitted =
        TransientStorageTest::testNestedContractsCall::abi_decode_returns(&return_value)
            .expect("failed to decode return");

    assert!(event_emitted, "testNestedContracts did not succeed as expected");
}

#[vm_test]
pub fn evm_transient_reentry_test(v: &dyn VM) {
    // Step 1: Create an EthAccount with an initial balance.
    let account = create_accounts(v, 1, &TokenAmount::from_whole(10_000))[0];

    // Step 2: Deploy the `TransientStorageTest` contract twice.
    let bytecode =
        hex::decode(include_str!("../../../actors/evm/tests/contracts/TransientStorageTest.hex"))
            .unwrap();

    let create_contract = |account: &Address| {
        let create_result = v
            .execute_message(
                account,
                &EAM_ACTOR_ADDR,
                &TokenAmount::zero(),
                fil_actor_eam::Method::CreateExternal as u64,
                Some(serialize_ok(&fil_actor_eam::CreateExternalParams(bytecode.clone()))),
            )
            .unwrap();

        assert!(
            create_result.code.is_success(),
            "failed to create the contract {}",
            create_result.message
        );

        let create_return: fil_actor_eam::CreateExternalReturn =
            create_result.ret.unwrap().deserialize().expect("failed to decode results");

        create_return.robust_address.unwrap()
    };

    // Deploy two instances of the contract.
    let contract_1_addr = create_contract(&account);
    let contract_2_addr = create_contract(&account);

    // Step 3: Call `testReentry` on the first contract, passing the address of the second contract.
    let nested_contract_id = v.resolve_id_address(&contract_2_addr).unwrap().id().unwrap();
    let nested_contract_address = id_to_eth(nested_contract_id);

    let call_params =
        TransientStorageTest::testReentryCall::new((nested_contract_address,)).abi_encode();

    let call_result = v
        .execute_message(
            &account,
            &contract_1_addr,
            &TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            Some(serialize_ok(&ContractParams(call_params.to_vec()))),
        )
        .unwrap();

    // Step 4: Check if the call was successful.
    assert!(call_result.code.is_success(), "testReentry call failed: {}", call_result.message);

    // Step 5: Deserialize and verify the return value.
    let BytesDe(return_value) =
        call_result.ret.unwrap().deserialize().expect("failed to deserialize results");

    let event_emitted = TransientStorageTest::testReentryCall::abi_decode_returns(&return_value)
        .expect("failed to decode return");

    assert!(event_emitted, "testReentry did not succeed as expected");
}
