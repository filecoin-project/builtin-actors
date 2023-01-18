use std::sync::Arc;

use ethers::core::types::Address as EthAddress;
use ethers::prelude::abigen;
use ethers::providers::Provider;
use fil_actor_evm::interpreter::U256;
use fil_actors_runtime::{
    test_utils::{ETHACCOUNT_ACTOR_CODE_ID, EVM_ACTOR_CODE_ID},
    EAM_ACTOR_ADDR, EAM_ACTOR_ID,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{strict_bytes, BytesDe, RawBytes};
use fvm_shared::{address::Address, econ::TokenAmount};
use fvm_shared::{ActorID, METHOD_SEND};
use num_traits::Zero;
use serde::{Deserialize, Serialize};
use test_vm::{
    util::{apply_ok, create_accounts},
    TEST_FAUCET_ADDR, VM,
};

// Generate a statically typed interface for the contracts.
abigen!(Recursive, "../actors/evm/tests/contracts/Recursive.abi");
abigen!(Factory, "../actors/evm/tests/contracts/Factory.abi");
abigen!(FactoryChild, "../actors/evm/tests/contracts/FactoryChild.abi");

fn id_to_eth(id: ActorID) -> EthAddress {
    let mut addr = [0u8; 20];
    addr[0] = 0xff;
    addr[12..].copy_from_slice(&id.to_be_bytes());
    EthAddress::from_slice(&addr)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct ContractParams(#[serde(with = "strict_bytes")] pub Vec<u8>);

#[test]
fn test_evm_call() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    let account = create_accounts(&v, 1, TokenAmount::from_whole(10_000))[0];

    let address = id_to_eth(account.id().unwrap());
    let (client, _mock) = Provider::mocked();
    let contract = Recursive::new(address, Arc::new(client));

    let bytecode =
        hex::decode(include_str!("../../actors/evm/tests/contracts/Recursive.hex")).unwrap();

    let create_result = v
        .apply_message(
            account,
            EAM_ACTOR_ADDR,
            TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            Some(fil_actor_eam::CreateExternalParams(bytecode)),
        )
        .unwrap();

    assert!(
        create_result.code.is_success(),
        "failed to create the new actor {}",
        create_result.message
    );

    let create_return: fil_actor_eam::CreateExternalReturn =
        create_result.ret.unwrap().deserialize().expect("failed to decode results");

    let contract_params = contract.enter().calldata().expect("should serialize");
    let call_result = v
        .apply_message(
            account,
            create_return.robust_address.unwrap(),
            TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            Some(ContractParams(contract_params.to_vec())),
        )
        .unwrap();
    assert!(call_result.code.is_success(), "failed to call the new actor {}", call_result.message);
    let BytesDe(return_value) =
        call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
    let evm_ret: u32 = contract
        .decode_output(&contract.enter().function.name, &return_value)
        .expect("failed to decode return");
    assert_eq!(0, evm_ret, "expected contract to return 0 on success");
}

#[test]
fn test_evm_create() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    let account = create_accounts(&v, 1, TokenAmount::from_whole(10_000))[0];

    let address = id_to_eth(account.id().unwrap());
    let (client, _mock) = Provider::mocked();
    let client = Arc::new(client);
    let factory = Factory::new(address, client.clone());
    let factory_child = FactoryChild::new(address, client);

    let bytecode =
        hex::decode(include_str!("../../actors/evm/tests/contracts/Lifecycle.hex")).unwrap();

    let create_result = v
        .apply_message(
            account,
            EAM_ACTOR_ADDR,
            TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            Some(fil_actor_eam::CreateExternalParams(bytecode)),
        )
        .unwrap();

    assert!(
        create_result.code.is_success(),
        "failed to create the new actor {}",
        create_result.message
    );

    let create_return: fil_actor_eam::CreateExternalReturn =
        create_result.ret.unwrap().deserialize().expect("failed to decode results");

    let child_addr: EthAddress = {
        let func = factory.create_2([0; 32], 42);
        let call_params = func.calldata().expect("should serialize");
        let call_result = v
            .apply_message(
                account,
                create_return.robust_address.unwrap(),
                TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(ContractParams(call_params.to_vec())),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.message
        );
        let BytesDe(return_value) =
            call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
        factory.decode_output(&func.function.name, &return_value).expect("failed to decode return")
    };

    let child_addr = Address::new_delegated(EAM_ACTOR_ID, &child_addr.0[..]).unwrap();

    {
        let func = factory_child.get_value();
        let call_params = func.calldata().expect("should serialize");
        let call_result = v
            .apply_message(
                account,
                child_addr,
                TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(ContractParams(call_params.to_vec())),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.message
        );
        let BytesDe(return_value) =
            call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
        let res: u32 = factory_child
            .decode_output(&func.function.name, &return_value)
            .expect("failed to decode return");
        assert_eq!(res, 42);
    };
}

#[test]
fn test_evm_eth_create_external() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    // create the EthAccount
    let eth_bits = hex_literal::hex!("FEEDFACECAFEBEEF000000000000000000000000");
    let eth_addr = Address::new_delegated(EAM_ACTOR_ID, &eth_bits).unwrap();
    apply_ok(
        &v,
        TEST_FAUCET_ADDR,
        eth_addr,
        TokenAmount::from_whole(10_000),
        METHOD_SEND,
        None::<RawBytes>,
    );
    let account = v.normalize_address(&eth_addr).unwrap();
    let mut actor = v.get_actor(account).unwrap();
    actor.code = *ETHACCOUNT_ACTOR_CODE_ID;
    v.set_actor(account, actor);

    // now create an empty contract
    let create_result = v
        .apply_message(
            account,
            EAM_ACTOR_ADDR,
            TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            Some(fil_actor_eam::CreateExternalParams(vec![])),
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

    let call_result = v
        .apply_message(
            account,
            robust_addr,
            TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            Some(ContractParams(vec![])),
        )
        .unwrap();
    assert!(call_result.code.is_success(), "failed to call the new actor {}", call_result.message);
}

#[test]
fn test_evm_empty_initcode() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    let account = create_accounts(&v, 1, TokenAmount::from_whole(10_000))[0];
    let create_result = v
        .apply_message(
            account,
            EAM_ACTOR_ADDR,
            TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            Some(fil_actor_eam::CreateExternalParams(vec![])),
        )
        .unwrap();

    assert!(
        create_result.code.is_success(),
        "failed to create the new actor {}",
        create_result.message
    );
}

#[test]
#[allow(non_snake_case)]
fn test_evm_staticcall() {
    // test scenarios:
    // one hop:
    // A -> staticcall -> B (read) OK
    // A -> staticcall -> B (write) FAIL
    // two hop sticky:
    // A -> staticcall -> B -> call -> C (read) OK
    // A -> staticcall -> B -> call -> C (write) FAIL

    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    let accounts = create_accounts(&v, 3, TokenAmount::from_whole(10_000));

    let bytecode =
        hex::decode(include_str!("../../actors/evm/tests/contracts/callvariants.hex")).unwrap();

    let created: Vec<_> = accounts
        .iter()
        .map(|account| {
            let create_result = v
                .apply_message(
                    *account,
                    EAM_ACTOR_ADDR,
                    TokenAmount::zero(),
                    fil_actor_eam::Method::CreateExternal as u64,
                    Some(fil_actor_eam::CreateExternalParams(bytecode.clone())),
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
                &v.get_actor(Address::new_id(create_return.actor_id)).unwrap().code,
                &*EVM_ACTOR_CODE_ID
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
            .apply_message(
                A_act,
                A_robust_addr,
                TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(ContractParams(params.to_vec())),
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
            .apply_message(
                A_act,
                A_robust_addr,
                TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(ContractParams(params.to_vec())),
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
            .apply_message(
                A_act,
                A_robust_addr,
                TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(ContractParams(params.to_vec())),
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
            .apply_message(
                A_act,
                A_robust_addr,
                TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(ContractParams(params.to_vec())),
            )
            .unwrap();
        assert_eq!(call_result.code.value(), 33, "static call mutation did not revert");
    }
}

#[test]
#[allow(non_snake_case)]
fn test_evm_delegatecall() {
    // test scenarios:
    // one hop:
    // A -> delegatecall -> B (read) OK
    // A -> delegatecall -> B (write) -> return (read) OK
    // two hop with sticky staticcall:
    // A -> staticcall -> B -> delegatecall -> C (read) OK
    // A -> staticcall -> B -> delegatecall -> C (write) FAIL

    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    let accounts = create_accounts(&v, 3, TokenAmount::from_whole(10_000));

    let bytecode =
        hex::decode(include_str!("../../actors/evm/tests/contracts/callvariants.hex")).unwrap();

    let created: Vec<_> = accounts
        .iter()
        .map(|account| {
            let create_result = v
                .apply_message(
                    *account,
                    EAM_ACTOR_ADDR,
                    TokenAmount::zero(),
                    fil_actor_eam::Method::CreateExternal as u64,
                    Some(fil_actor_eam::CreateExternalParams(bytecode.clone())),
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
                &v.get_actor(Address::new_id(create_return.actor_id)).unwrap().code,
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
            .apply_message(
                A_act,
                A_robust_addr,
                TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(ContractParams(params.to_vec())),
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
            .apply_message(
                A_act,
                A_robust_addr,
                TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(ContractParams(params.to_vec())),
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
            .apply_message(
                A_act,
                A_robust_addr,
                value.clone(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(ContractParams(params.to_vec())),
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

#[test]
#[allow(non_snake_case)]
fn test_evm_staticcall_delegatecall() {
    // test scenarios:
    // one hop:
    // A -> delegatecall -> B (read) OK
    // A -> delegatecall -> B (write) -> return (read) OK
    // two hop with sticky staticcall:
    // A -> staticcall -> B -> delegatecall -> C (read) OK
    // A -> staticcall -> B -> delegatecall -> C (write) FAIL

    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    let accounts = create_accounts(&v, 3, TokenAmount::from_whole(10_000));

    let bytecode =
        hex::decode(include_str!("../../actors/evm/tests/contracts/callvariants.hex")).unwrap();

    let created: Vec<_> = accounts
        .iter()
        .map(|account| {
            let create_result = v
                .apply_message(
                    *account,
                    EAM_ACTOR_ADDR,
                    TokenAmount::zero(),
                    fil_actor_eam::Method::CreateExternal as u64,
                    Some(fil_actor_eam::CreateExternalParams(bytecode.clone())),
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
                &v.get_actor(Address::new_id(create_return.actor_id)).unwrap().code,
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
            .apply_message(
                A_act,
                A_robust_addr,
                TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(ContractParams(params.to_vec())),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.message
        );
        let BytesDe(return_value) =
            call_result.ret.unwrap().deserialize().expect("failed to deserialize results");
        //assert_eq!(&return_value[12..], &created[1].eth_address.0);
        println!("return {:?}", return_value)
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
            .apply_message(
                A_act,
                A_robust_addr,
                TokenAmount::zero(),
                fil_actor_evm::Method::InvokeContract as u64,
                Some(ContractParams(params.to_vec())),
            )
            .unwrap();
        assert_eq!(call_result.code.value(), 33, "static call mutation did not revert");
    }
}

#[test]
fn test_evm_init_revert_data() {
    let store = MemoryBlockstore::new();
    let v = VM::new_with_singletons(&store);

    let account = create_accounts(&v, 1, TokenAmount::from_whole(10_000))[0];
    let create_result = v
        .apply_message(
            account,
            EAM_ACTOR_ADDR,
            TokenAmount::zero(),
            fil_actor_eam::Method::CreateExternal as u64,
            // init code:
            // PUSH1 0x42; PUSH1 0x0; MSTORE;
            // PUSH1 0x20; PUSH1 0x0; REVERT
            Some(fil_actor_eam::CreateExternalParams(vec![
                0x60, 0x42, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xfd,
            ])),
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
