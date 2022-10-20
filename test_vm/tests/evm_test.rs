use std::sync::Arc;

use ethers::core::types::Address as EthAddress;
use ethers::prelude::abigen;
use ethers::providers::Provider;
use fil_actors_runtime::{test_utils::EVM_ACTOR_CODE_ID, EAM_ACTOR_ADDR};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{strict_bytes, BytesDe, Cbor};
use fvm_shared::ActorID;
use fvm_shared::{address::Address, econ::TokenAmount};
use num_traits::Zero;
use serde::{Deserialize, Serialize};
use test_vm::{util::create_accounts, VM};

// Generate a statically typed interface for the contract.
abigen!(Recursive, "../actors/evm/tests/contracts/Recursive.abi");

fn id_to_eth(id: ActorID) -> EthAddress {
    let mut addr = [0u8; 20];
    addr[0] = 0xff;
    addr[12..].copy_from_slice(&id.to_be_bytes());
    EthAddress::from_slice(&addr)
}

// TODO: we should move this somewhere else, or just find a way to avoid this. Unfortunately, "BytesSer" doesn't implemenet `Cbor`.
// Really, we should consider getting rid of `Cbor` entirely.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct ContractParams(#[serde(with = "strict_bytes")] pub Vec<u8>);

impl Cbor for ContractParams {}

#[test]
fn test_evm_lifecycle() {
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
            fil_actor_eam::Method::Create2 as u64,
            fil_actor_eam::Create2Params { initcode: bytecode, salt: [0u8; 32] },
        )
        .unwrap();

    assert!(
        create_result.code.is_success(),
        "failed to create the new actor {}",
        create_result.error_message.unwrap()
    );

    let create_return: fil_actor_eam::Create2Return =
        create_result.ret.deserialize().expect("failed to decode results");

    let contract_params = contract.enter().calldata().expect("should serialize");
    let call_result = v
        .apply_message(
            account,
            create_return.robust_address,
            TokenAmount::zero(),
            fil_actor_evm::Method::InvokeContract as u64,
            ContractParams(contract_params.to_vec()),
        )
        .unwrap();
    assert!(
        call_result.code.is_success(),
        "failed to call the new actor {}",
        call_result.error_message.unwrap()
    );
    let BytesDe(return_value) =
        call_result.ret.deserialize().expect("failed to deserialize results");
    let evm_ret: u32 = contract
        .decode_output(&contract.enter().function.name, &return_value)
        .expect("failed to decode return");
    assert_eq!(0, evm_ret, "expected contract to return 0 on success");
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
                    fil_actor_eam::Method::Create2 as u64,
                    fil_actor_eam::Create2Params { initcode: bytecode.clone(), salt: [0u8; 32] },
                )
                .unwrap();

            assert!(
                create_result.code.is_success(),
                "failed to create the new actor {}",
                create_result.error_message.unwrap()
            );

            let create_return: fil_actor_eam::Create2Return =
                create_result.ret.deserialize().expect("failed to decode results");

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
        let A_act = accounts[0].clone();
        let A_robust_addr = created[0].robust_address.clone();
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
                ContractParams(params.to_vec()),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.error_message.unwrap()
        );
        let BytesDe(return_value) =
            call_result.ret.deserialize().expect("failed to deserialize results");
        assert_eq!(&return_value[12..], &created[1].eth_address.0);
    }

    // A -> staticcall -> B (write) FAIL
    {
        let A_act = accounts[0].clone();
        let A_robust_addr = created[0].robust_address.clone();
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
                ContractParams(params.to_vec()),
            )
            .unwrap();
        assert!(!call_result.code.is_success(), "static call mutation succeeded");
    }

    // A -> staticcall -> B -> call -> C (read) OK
    {
        let A_act = accounts[0].clone();
        let A_robust_addr = created[0].robust_address.clone();
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
                ContractParams(params.to_vec()),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.error_message.unwrap()
        );
        let BytesDe(return_value) =
            call_result.ret.deserialize().expect("failed to deserialize results");
        assert_eq!(&return_value[12..], &created[2].eth_address.0);
    }

    // A -> staticcall -> B -> call -> C (write) FAIL
    {
        let A_act = accounts[0].clone();
        let A_robust_addr = created[0].robust_address.clone();
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
                ContractParams(params.to_vec()),
            )
            .unwrap();
        assert!(!call_result.code.is_success(), "static call mutation succeeded",);
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
                    fil_actor_eam::Method::Create2 as u64,
                    fil_actor_eam::Create2Params { initcode: bytecode.clone(), salt: [0u8; 32] },
                )
                .unwrap();

            assert!(
                create_result.code.is_success(),
                "failed to create the new actor {}",
                create_result.error_message.unwrap()
            );

            let create_return: fil_actor_eam::Create2Return =
                create_result.ret.deserialize().expect("failed to decode results");

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
        let A_act = accounts[0].clone();
        let A_robust_addr = created[0].robust_address.clone();
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
                ContractParams(params.to_vec()),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.error_message.unwrap()
        );
        let BytesDe(return_value) =
            call_result.ret.deserialize().expect("failed to deserialize results");
        assert_eq!(&return_value[12..], &created[0].eth_address.0);
    }

    // A -> delegatecall -> B (write) -> return (read) OK
    {
        let A_act = accounts[0].clone();
        let A_robust_addr = created[0].robust_address.clone();
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
                ContractParams(params.to_vec()),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.error_message.unwrap()
        );
        let BytesDe(return_value) =
            call_result.ret.deserialize().expect("failed to deserialize results");
        assert_eq!(&return_value[28..], &[0xff, 0xff, 0xff, 0x42]);
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
                    fil_actor_eam::Method::Create2 as u64,
                    fil_actor_eam::Create2Params { initcode: bytecode.clone(), salt: [0u8; 32] },
                )
                .unwrap();

            assert!(
                create_result.code.is_success(),
                "failed to create the new actor {}",
                create_result.error_message.unwrap()
            );

            let create_return: fil_actor_eam::Create2Return =
                create_result.ret.deserialize().expect("failed to decode results");

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
        let A_act = accounts[0].clone();
        let A_robust_addr = created[0].robust_address.clone();
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
                ContractParams(params.to_vec()),
            )
            .unwrap();
        assert!(
            call_result.code.is_success(),
            "failed to call the new actor {}",
            call_result.error_message.unwrap()
        );
        let BytesDe(return_value) =
            call_result.ret.deserialize().expect("failed to deserialize results");
        //assert_eq!(&return_value[12..], &created[1].eth_address.0);
        println!("return {:?}", return_value)
    }

    // A -> staticcall -> B -> delegatecall -> C (write) FAIL
    {
        let A_act = accounts[0].clone();
        let A_robust_addr = created[0].robust_address.clone();
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
                ContractParams(params.to_vec()),
            )
            .unwrap();
        assert!(!call_result.code.is_success(), "static call mutation succeeded",);
    }
}
