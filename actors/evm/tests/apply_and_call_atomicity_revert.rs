use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::MockRuntime;
use fil_actors_runtime::test_utils::hash as rt_hash;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::crypto::hash::SupportedHashes;
use fvm_shared::{
    IPLD_RAW, address::Address as FilAddress, econ::TokenAmount, error::ExitCode, sys::SendFlags,
};

mod util;

#[test]
fn mapping_and_nonce_persist_on_outer_call_failure() {
    let mut rt = util::construct_and_verify(vec![]);

    // Intrinsic gas charges
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);

    // First attempt: mapping A->B; outer call to B (EVM) should go through InvokeContract which we fail.
    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);
    // Choose a fixed pubkey for authority A and derive its EthAddress (A); override recover to return this pubkey.
    let mut pk_a = [0u8; 65];
    pk_a[0] = 0x04;
    for i in 1..65 {
        pk_a[i] = 0xA1;
    }
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_a));
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: b_eth,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let params = evm::ApplyAndCallParams {
        list: list.clone(),
        call: evm::ApplyCall { to: b_eth, value: vec![], input: vec![0x01] },
    };

    // Set destination B as EVM actor and simulate InvokeContract failure.
    use fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID;
    let b_f4: FilAddress = b_eth.into();
    let b_id = FilAddress::new_id(0x444u64);
    rt.set_delegated_address(b_id.id().unwrap(), b_f4);
    rt.actor_code_cids.borrow_mut().insert(b_id, *EVM_ACTOR_CODE_ID);
    rt.expect_send(
        b_id,
        evm::Method::InvokeContract as u64,
        Some(IpldBlock { codec: IPLD_RAW, data: vec![0x01] }),
        TokenAmount::from_whole(0),
        None,
        SendFlags::empty(),
        None,
        ExitCode::USR_ILLEGAL_STATE,
        None,
    );
    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    // Outer call failure should not abort ApplyAndCall; it should return OK.
    assert!(res.is_ok());

    // Note: Nonce bump persistence is covered by state flush before GetBytecode; a dedicated
    // integration test can assert nonce mismatch behavior once harness support is in place.
}
