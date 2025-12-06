mod asm;

#[allow(unused_imports)]
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::uints::U256;
use fil_actors_runtime::test_utils::{self, PLACEHOLDER_ACTOR_CODE_ID};
#[allow(unused_imports)]
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address as FilAddress;
use fvm_shared::crypto::hash::SupportedHashes;

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

    // Ensure A resolves to an Account/Placeholder type (EOA) in the runtime.
    let authority_fil_addr: FilAddress = authority.into();
    let a_id = FilAddress::new_id(0xABCDu64);
    rt.set_delegated_address(a_id.id().unwrap(), authority_fil_addr);
    rt.set_address_actor_type(a_id, *PLACEHOLDER_ACTOR_CODE_ID);

    // Delegate B is this EVM actor (receiver) with known ETH f4 address.
    let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);

    // In the new architecture, EXTCODE* consults the runtime helper for delegation mapping.
    // Program the helper to reflect A -> B so pointer projection engages in tests.
    rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk));
    rt.set_eth_delegate_to(a_id.id().unwrap(), b_eth.0);

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
