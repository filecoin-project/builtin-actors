use std::sync::Arc;

use ethers::core::types::Address as EthAddress;
use ethers::prelude::abigen;
use ethers::providers::Provider;
use fil_actors_runtime::EAM_ACTOR_ADDR;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::{strict_bytes, BytesDe, Cbor};
use fvm_shared::econ::TokenAmount;
use fvm_shared::ActorID;
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
