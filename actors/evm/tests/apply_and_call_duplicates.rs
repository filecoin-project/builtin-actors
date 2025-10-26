use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::MockRuntime;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::error::ExitCode;

mod util;

#[test]
fn apply_and_call_rejects_duplicate_authorities() {
    let rt = util::construct_and_verify(vec![]);

    // Expect intrinsic gas charges for 2 tuples.
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE * 2);

    // Two tuples for the same authority.
    let authority = EthAddress(hex_literal::hex!("00112233445566778899aabbccddeeff00112233"));
    let list = vec![
        evm::DelegationParam { chain_id: 0, address: authority, nonce: 0, y_parity: 0, r: vec![1u8;32], s: vec![1u8;32] },
        evm::DelegationParam { chain_id: 0, address: authority, nonce: 1, y_parity: 0, r: vec![1u8;32], s: vec![1u8;32] },
    ];
    let params = evm::ApplyAndCallParams { list, call: evm::ApplyCall { to: authority, value: vec![], input: vec![] } };

    rt.expect_validate_caller_any();
    let res = rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    );
    assert!(res.is_err());
    assert_eq!(res.err().unwrap().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

