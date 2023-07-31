use fil_actors_evm_shared::uints::U256;
use fil_actors_integration_tests::tests::{
    evm_call_test, evm_create_test, evm_init_revert_data_test, evm_staticcall_delegatecall_test,
    id_to_eth,
};
use fil_actors_integration_tests::util::create_accounts;
use fil_actors_runtime::{
    test_utils::ETHACCOUNT_ACTOR_CODE_ID, test_utils::EVM_ACTOR_CODE_ID, EAM_ACTOR_ADDR,
    EAM_ACTOR_ID,
};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::{strict_bytes, BytesDe, RawBytes};
use fvm_shared::METHOD_SEND;
use fvm_shared::{address::Address, econ::TokenAmount};
use num_traits::Zero;
use serde::{Deserialize, Serialize};
use test_vm::{TestVM, TEST_FAUCET_ADDR};
use vm_api::util::{apply_ok, serialize_ok};
use vm_api::VM;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct ContractParams(#[serde(with = "strict_bytes")] pub Vec<u8>);

#[test]
fn evm_call() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_call_test(&v);
}

#[test]
fn evm_create() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_create_test(&v);
}

#[test]
fn evm_eth_create_external() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_eth_create_external_test(&v, &v);
}

// Concrete use of TestVM is required here to run `set_actor`
// TODO(alexytsu): Removing it will depend on https://github.com/filecoin-project/builtin-actors/issues/1297
fn evm_eth_create_external_test(v: &dyn VM, _v_concrete: &TestVM<MemoryBlockstore>) {
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
    _v_concrete.set_actor(&account, actor);

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

#[test]
fn evm_empty_initcode() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);

    evm_empty_initcode_test(&v);
}

fn evm_empty_initcode_test(v: &dyn VM) {
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

#[test]
fn evm_staticcall() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);

    evm_staticcall_test(&v);
}

#[allow(non_snake_case)]
fn evm_staticcall_test(v: &dyn VM) {
    // test scenarios:
    // one hop:
    // A -> staticcall -> B (read) OK
    // A -> staticcall -> B (write) FAIL
    // two hop sticky:
    // A -> staticcall -> B -> call -> C (read) OK
    // A -> staticcall -> B -> call -> C (write) FAIL
    let accounts = create_accounts(v, 3, &TokenAmount::from_whole(10_000));

    let bytecode =
        hex::decode(include_str!("../../actors/evm/tests/contracts/callvariants.hex")).unwrap();

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

#[test]
fn evm_delegatecall() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);

    evm_delegatecall_test(&v);
}

#[allow(non_snake_case)]
fn evm_delegatecall_test(v: &dyn VM) {
    // test scenarios:
    // one hop:
    // A -> delegatecall -> B (read) OK
    // A -> delegatecall -> B (write) -> return (read) OK
    // two hop with sticky staticcall:
    // A -> staticcall -> B -> delegatecall -> C (read) OK
    // A -> staticcall -> B -> delegatecall -> C (write) FAIL
    let accounts = create_accounts(v, 3, &TokenAmount::from_whole(10_000));

    let bytecode =
        hex::decode(include_str!("../../actors/evm/tests/contracts/callvariants.hex")).unwrap();

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

#[test]
fn evm_staticcall_delegatecall() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_staticcall_delegatecall_test(&v);
}

#[test]
fn evm_init_revert_data() {
    let store = MemoryBlockstore::new();
    let v = TestVM::<MemoryBlockstore>::new_with_singletons(&store);
    evm_init_revert_data_test(&v);
}
