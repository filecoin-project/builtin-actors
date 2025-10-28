mod asm;

use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::test_utils::{self, PLACEHOLDER_ACTOR_CODE_ID};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::crypto::hash::SupportedHashes;
use fvm_shared::version::NetworkVersion;

mod util;

#[test]
fn eoa_extcode_pointer_semantics_post_activation() {
    // Set up deterministic authority address A from a fixed pubkey.
    let mut pk = [0u8; 65];
    pk[0] = 0x04;
    for b in pk.iter_mut().skip(1) {
        *b = 0xA9;
    }
    let (keccak_a, _) = test_utils::hash(SupportedHashes::Keccak256, &pk[1..]);
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);
    let authority = EthAddress(a20);

    // Contract that exposes:
    //  - EXTCODESIZE(A) => returns 32-byte value
    //  - EXTCODECOPY(A,0,0,23) => copies pointer code into memory and returns 23 bytes
    //  - EXTCODEHASH(A) => returns 32-byte keccak256 of pointer code
    let init = "";
    let body = format!(
        r#"
%dispatch_begin()
%dispatch(0x00, extsize_a)
%dispatch(0x01, extcopy_a)
%dispatch(0x02, exthash_a)
%dispatch_end()

extsize_a:
  jumpdest
  push20 0x{a}
  extcodesize
  %return_stack_word()

extcopy_a:
  jumpdest
  # EXTCODECOPY(A, dst=0, offset=0, size=23) then return 23 bytes
  push1 0x17
  push1 0x00
  push1 0x00
  push20 0x{a}
  extcodecopy
  push1 0x17
  push1 0x00
  return

exthash_a:
  jumpdest
  push20 0x{a}
  extcodehash
  %return_stack_word()
"#,
        a = hex::encode_upper(authority.as_ref())
    );
    let bytecode = asm::new_contract("eoa-pointer", init, &body).unwrap();

    let mut rt = util::construct_and_verify(bytecode);

    // Activate NV gate for 7702 features.
    rt.set_network_version(NetworkVersion::V16);

    // Ensure A resolves to an Account/Placeholder type (EOA) in the runtime.
    let a_f4: FilAddress = authority.into();
    let a_id = FilAddress::new_id(0xABCDu64);
    rt.set_delegated_address(a_id.id().unwrap(), a_f4);
    rt.set_address_actor_type(a_id, *PLACEHOLDER_ACTOR_CODE_ID);

    // Delegate B is this EVM actor (receiver) with known ETH f4 address.
    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);

    // Apply mapping A -> B via ApplyAndCall with a no-op outer call.
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk));
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: b_eth,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let to_other = EthAddress::from_id(0xB0B0);
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: to_other, value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    )
    .unwrap();

    // EXTCODESIZE(A) == 23
    let size_out = util::invoke_contract(&rt, &util::dispatch_num_word(0x00));
    rt.verify();
    assert_eq!(U256::from_big_endian(&size_out), U256::from(23u64));
    rt.reset();

    // EXTCODECOPY(A, 0, 0, 23) yields 23-byte pointer code = magic||version||B
    let copy_out = util::invoke_contract(&rt, &util::dispatch_num_word(0x01));
    rt.verify();
    assert_eq!(copy_out.len(), 23);
    let mut expected = Vec::with_capacity(23);
    expected.extend_from_slice(&fil_actors_evm_shared::eip7702::EIP7702_MAGIC);
    expected.push(fil_actors_evm_shared::eip7702::EIP7702_VERSION);
    expected.extend_from_slice(b_eth.as_ref());
    assert_eq!(copy_out, expected);

    // EXTCODEHASH(A) equals keccak256(pointer_code)
    let hash_out = util::invoke_contract(&rt, &util::dispatch_num_word(0x02));
    rt.verify();
    // compute expected keccak256 over the 23-byte pointer code
    let (expected_hash, written) = test_utils::hash(SupportedHashes::Keccak256, &expected);
    assert_eq!(hash_out.len(), 32);
    assert_eq!(hash_out, expected_hash[..written]);
}

#[test]
fn eoa_extcode_pointer_semantics_nv_sanity() {
    // Same setup as post-activation, but verify pre-activation does not expose pointer semantics.
    let mut pk = [0u8; 65];
    pk[0] = 0x04;
    for b in pk.iter_mut().skip(1) {
        *b = 0xA9;
    }
    let (keccak_a, _) = test_utils::hash(SupportedHashes::Keccak256, &pk[1..]);
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);
    let authority = EthAddress(a20);

    let init = "";
    let body = format!(
        r#"
%dispatch_begin()
%dispatch(0x00, extsize_a)
%dispatch(0x01, extcopy_a)
%dispatch_end()

extsize_a:
  jumpdest
  push20 0x{a}
  extcodesize
  %return_stack_word()

extcopy_a:
  jumpdest
  push1 0x17
  push1 0x00
  push1 0x00
  push20 0x{a}
  extcodecopy
  push1 0x17
  push1 0x00
  return
"#,
        a = hex::encode_upper(authority.as_ref())
    );
    let bytecode = asm::new_contract("eoa-pointer-nv", init, &body).unwrap();
    let mut rt = util::construct_and_verify(bytecode);

    // Pre-activation (NV < NV_EIP_7702)
    // Choose a version lower than the activation version used elsewhere (V16).
    // If V15 is not available in target, adjust to the immediate predecessor.
    rt.set_network_version(NetworkVersion::V15);

    // Ensure A resolves to an Account-like actor
    let a_f4: FilAddress = authority.into();
    let a_id = FilAddress::new_id(0xABCEu64);
    rt.set_delegated_address(a_id.id().unwrap(), a_f4);
    rt.set_address_actor_type(a_id, *PLACEHOLDER_ACTOR_CODE_ID);

    // Map A -> B (mapping should be ignored pre-activation)
    const GAS_BASE_APPLY7702: i64 = 0;
    const GAS_PER_AUTH_TUPLE: i64 = 10_000;
    rt.expect_gas_charge(GAS_BASE_APPLY7702);
    rt.expect_gas_charge(GAS_PER_AUTH_TUPLE);
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk));
    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: b_eth,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let to_other = EthAddress::from_id(0xB0B0);
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: to_other, value: vec![], input: vec![] },
    };
    rt.expect_validate_caller_any();
    rt.call::<evm::EvmContractActor>(
        evm::Method::ApplyAndCall as u64,
        IpldBlock::serialize_dag_cbor(&params).unwrap(),
    )
    .unwrap();

    // EXTCODESIZE(A) should NOT report pointer size (23) pre-activation
    let size_out = util::invoke_contract(&rt, &util::dispatch_num_word(0x00));
    rt.verify();
    assert_ne!(U256::from_big_endian(&size_out), U256::from(23u64));

    // EXTCODECOPY(A,0,0,23) should NOT match pointer bytes pre-activation
    let copy_out = util::invoke_contract(&rt, &util::dispatch_num_word(0x01));
    rt.verify();
    let mut expected = Vec::with_capacity(23);
    expected.extend_from_slice(&fil_actors_evm_shared::eip7702::EIP7702_MAGIC);
    expected.push(fil_actors_evm_shared::eip7702::EIP7702_VERSION);
    expected.extend_from_slice(b_eth.as_ref());
    assert_ne!(copy_out, expected);
}
