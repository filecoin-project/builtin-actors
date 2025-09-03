use cid::Cid;
use fil_actor_evm as evm;
use fil_actor_evm::EvmContractActor;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::runtime::{Primitives, EMPTY_ARR_CID};
use fil_actors_runtime::test_utils::{EVM_ACTOR_CODE_ID, MockRuntime};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sys::SendFlags;

#[test]
fn invoke_as_eoa_mounts_and_persists_storage_root() {
    let rt = MockRuntime { receiver: FilAddress::new_id(1000), ..Default::default() };
    rt.in_call.replace(true);
    rt.set_address_actor_type(rt.receiver, *EVM_ACTOR_CODE_ID);

    // Bytecode: minimal STOP (0x00). Store it and use its CID.
    let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
    rt.store.put_keyed(&bytecode_cid, &[0x00]).unwrap();

    // Expect Delegator GetStorageRoot(authority) -> Some(EMPTY_ARR_CID) so mount succeeds,
    // and we can predict the same root is flushed back when no writes occur.
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("GetStorageRoot"),
        IpldBlock::serialize_cbor(&crate_get_params(EthAddress::from_id(5001))).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&crate_get_return(Some(EMPTY_ARR_CID))).unwrap(),
        ExitCode::OK,
        None,
    );

    // Expect Delegator PutStorageRoot(authority, root) with some CID; return OK.
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("PutStorageRoot"),
        IpldBlock::serialize_dag_cbor(&crate_put_params(EthAddress::from_id(5001), EMPTY_ARR_CID)).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::empty(),
        None,
        ExitCode::OK,
        None,
    );

    // Expect caller validation: InvokeAsEoa requires self-call.
    rt.expect_validate_caller_addr(vec![rt.receiver]);
    rt.set_caller(*EVM_ACTOR_CODE_ID, rt.receiver);

    // Call InvokeAsEoa.
    let params = evm::EoaInvokeParams {
        code: bytecode_cid,
        input: vec![],
        caller: EthAddress::from_id(4001),
        receiver: EthAddress::from_id(5001),
        value: TokenAmount::from_whole(0),
    };
    let res = rt.call::<EvmContractActor>(evm::Method::InvokeAsEoa as u64, IpldBlock::serialize_dag_cbor(&params).unwrap()).unwrap();
    assert!(res.is_some());
    let out: evm::InvokeContractReturn = res.unwrap().deserialize().unwrap();
    assert_eq!(out.output_data, Vec::<u8>::new());
}

#[test]
fn invoke_as_eoa_persists_and_reads_root_across_calls() {
    let rt = MockRuntime { receiver: FilAddress::new_id(1001), ..Default::default() };
    rt.in_call.replace(true);
    rt.set_address_actor_type(rt.receiver, *EVM_ACTOR_CODE_ID);

    // Bytecode: minimal STOP (0x00)
    let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
    rt.store.put_keyed(&bytecode_cid, &[0x00]).unwrap();

    // 1st call: mount None -> persist EMPTY_ARR_CID
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("GetStorageRoot"),
        IpldBlock::serialize_cbor(&crate_get_params(EthAddress::from_id(6001))).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&crate_get_return(None)).unwrap(),
        ExitCode::OK,
        None,
    );
    rt.expect_validate_caller_addr(vec![rt.receiver]);
    rt.set_caller(*EVM_ACTOR_CODE_ID, rt.receiver);
    let params = evm::EoaInvokeParams {
        code: bytecode_cid,
        input: vec![],
        caller: EthAddress::from_id(4002),
        receiver: EthAddress::from_id(6001),
        value: TokenAmount::from_whole(0),
    };
    // Expect Persist
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("PutStorageRoot"),
        IpldBlock::serialize_dag_cbor(&crate_put_params(EthAddress::from_id(6001), EMPTY_ARR_CID)).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::empty(),
        None,
        ExitCode::OK,
        None,
    );
    let _ = rt.call::<EvmContractActor>(evm::Method::InvokeAsEoa as u64, IpldBlock::serialize_dag_cbor(&params).unwrap()).unwrap();
    rt.verify();

    // 2nd call: read back previously written root and persist again
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("GetStorageRoot"),
        IpldBlock::serialize_cbor(&crate_get_params(EthAddress::from_id(6001))).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::READ_ONLY,
        IpldBlock::serialize_cbor(&crate_get_return(Some(EMPTY_ARR_CID))).unwrap(),
        ExitCode::OK,
        None,
    );
    rt.expect_validate_caller_addr(vec![rt.receiver]);
    rt.set_caller(*EVM_ACTOR_CODE_ID, rt.receiver);
    rt.expect_send(
        fil_actors_runtime::DELEGATOR_ACTOR_ADDR,
        frc42_dispatch::method_hash!("PutStorageRoot"),
        IpldBlock::serialize_dag_cbor(&crate_put_params(EthAddress::from_id(6001), EMPTY_ARR_CID)).unwrap(),
        TokenAmount::from_whole(0),
        None,
        SendFlags::empty(),
        None,
        ExitCode::OK,
        None,
    );
    let _ = rt.call::<EvmContractActor>(evm::Method::InvokeAsEoa as u64, IpldBlock::serialize_dag_cbor(&params).unwrap()).unwrap();
    rt.verify();
}

// Helpers

#[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
struct GetStorageRootParamsLocal { authority: EthAddress }
#[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
struct GetStorageRootReturnLocal { root: Option<Cid> }
#[derive(fvm_ipld_encoding::serde::Serialize, fvm_ipld_encoding::serde::Deserialize)]
struct PutStorageRootParamsLocal { authority: EthAddress, root: Cid }

fn crate_get_params(authority: EthAddress) -> GetStorageRootParamsLocal { GetStorageRootParamsLocal { authority } }
fn crate_get_return(root: Option<Cid>) -> GetStorageRootReturnLocal { GetStorageRootReturnLocal { root } }
fn crate_put_params(authority: EthAddress, root: Cid) -> PutStorageRootParamsLocal { PutStorageRootParamsLocal { authority, root } }
