mod asm;

use fil_actor_eam as eam;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::EAM_ACTOR_ADDR;
use fil_actors_runtime::test_utils::{self, PLACEHOLDER_ACTOR_CODE_ID};
use fvm_ipld_encoding::{BytesDe, BytesSer};
use fvm_shared::address::Address as FilAddress;
use fvm_shared::bigint::Zero;
use fvm_shared::crypto::hash::SupportedHashes;
use fvm_shared::econ::TokenAmount;
use test_vm::TestVM;
use vm_api::util::serialize_ok;
use vm_api::{VM, new_actor};

/// Fixed uncompressed secp256k1 pubkey (65 bytes) used by the test runtime's
/// recover_secp_public_key override. First byte 0x04 indicates uncompressed.
const FIXED_PUBKEY: [u8; 65] = {
    let mut pk = [0u8; 65];
    // 0x04 + 64 bytes body
    // We fill the body with a constant pattern; the derived ETH address is
    // computed from keccak(pk[1..]) inside the test.
    pk[0] = 0x04;
    let mut i = 1;
    while i < 65 {
        pk[i] = 0xA9;
        i += 1;
    }
    pk
};

fn create_evm_actor(v: &TestVM, from: &FilAddress, initcode: Vec<u8>) -> (FilAddress, EthAddress) {
    // Call EAM.CreateExternal to deploy an EVM actor with an ETH delegated address.
    let ret: eam::CreateExternalReturn = v
        .execute_message(
            from,
            &EAM_ACTOR_ADDR,
            &TokenAmount::zero(),
            eam::Method::CreateExternal as u64,
            Some(serialize_ok(&eam::CreateExternalParams(initcode))),
        )
        .unwrap()
        .ret
        .unwrap()
        .deserialize()
        .unwrap();
    (FilAddress::new_id(ret.actor_id), ret.eth_address)
}

#[test]
fn eoa_pointer_mapping_should_be_global_but_is_local_today() {
    // Build a TestVM with singletons (System, Init, EAM, etc.).
    let store = fil_actors_runtime::test_blockstores::MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);

    // Use faucet as the sender of deployment transactions.
    let from = test_vm::TEST_FAUCET_ADDR;

    // Derive a deterministic authority (EOA) address A from a fixed pubkey.
    let (keccak_a, _) = test_utils::hash(SupportedHashes::Keccak256, &FIXED_PUBKEY[1..]);
    let mut a20 = [0u8; 20];
    a20.copy_from_slice(&keccak_a[12..32]);
    let authority_a = EthAddress(a20);
    let a_f4: FilAddress = FilAddress::new_delegated(fil_actors_runtime::EAM_ACTOR_ID, &a20)
        .expect("construct delegated f4 address for A");

    // Ensure A resolves to an Account/Placeholder actor so EXTCODESIZE(A) uses pointer semantics.
    let a_id = FilAddress::new_id(0xABCD);
    v.set_actor(
        &a_id,
        new_actor(
            *PLACEHOLDER_ACTOR_CODE_ID,
            fil_actors_runtime::runtime::EMPTY_ARR_CID,
            0,
            TokenAmount::zero(),
            Some(a_f4),
        ),
    );

    // Deploy three EVM actors via EAM:
    // - M (manager): where we apply A->D mapping via ApplyAndCall.
    // - C (caller): runs bytecode to query EXTCODESIZE(A).
    // - D (delegate): the delegate address for mapping; code contents are irrelevant for EXTCODESIZE.
    let (m_id, _m_eth) = create_evm_actor(&v, &from, Vec::new());

    // Caller C: contract that returns EXTCODESIZE(A) as a 32-byte word.
    let init = "";
    let body = format!(
        r#"
%dispatch_begin()
%dispatch(0x00, extsize_a)
%dispatch_end()

extsize_a:
  jumpdest
  push20 0x{a}
  extcodesize
  %return_stack_word()
"#,
        a = hex::encode_upper(authority_a.as_ref())
    );
    let c_initcode = asm::new_contract("pointer-extcodesize", init, &body).unwrap();
    let (c_id, _c_eth) = create_evm_actor(&v, &from, c_initcode);

    // Delegate D: empty code is fine.
    let (_d_id, d_eth) = create_evm_actor(&v, &from, Vec::new());

    // Override recover_secp_public_key to return our fixed pubkey so the actor recovers A.
    v.mut_primitives().override_recover_secp_public_key(|_, _| Ok(FIXED_PUBKEY));

    // Apply mapping A -> D on actor M via ApplyAndCall with a no-op outer call.
    let list = vec![evm::DelegationParam {
        chain_id: 0,
        address: d_eth,
        nonce: 0,
        y_parity: 0,
        r: vec![1u8; 32],
        s: vec![1u8; 32],
    }];
    let params = evm::ApplyAndCallParams {
        list,
        call: evm::ApplyCall { to: EthAddress::from_id(0xB0B0), value: vec![], input: vec![] },
    };
    // Any account may call; INIT actor address is irrelevant here, but we keep consistency.
    let _ = v
        .execute_message(
            &from,
            &m_id,
            &TokenAmount::zero(),
            evm::Method::ApplyAndCall as u64,
            Some(serialize_ok(&params)),
        )
        .unwrap();

    // Now, from a different EVM actor C, query EXTCODESIZE(A).
    // Per desired global semantics, this should be 23.
    // Under current per-actor mapping, this will be 0 because C's local map is empty.
    let input = [
        // dispatch selector 0x00
        {
            let mut w = [0u8; 32];
            w[3] = 0x00;
            w
        }
        .to_vec(),
    ]
    .concat();
    let res = v
        .execute_message(
            &from,
            &c_id,
            &TokenAmount::zero(),
            evm::Method::InvokeContract as u64,
            Some(serialize_ok(&BytesSer(&input))),
        )
        .unwrap()
        .ret
        .unwrap();
    let BytesDe(out) = res.deserialize::<BytesDe>().unwrap();

    use fil_actors_evm_shared::uints::U256;
    let size = U256::from_big_endian(&out);

    // Failing assertion today: proves mapping is not global.
    assert_eq!(
        size,
        U256::from(23u64),
        "EXTCODESIZE(A) should see pointer code when mapping is global"
    );
}
