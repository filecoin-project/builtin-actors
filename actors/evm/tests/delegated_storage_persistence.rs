use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::MockRuntime;
use fvm_ipld_blockstore::{Block, Blockstore};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{IPLD_RAW, econ::TokenAmount};

mod util;

fn put_code(rt: &MockRuntime, code: &[u8]) -> Cid {
    use multihash_codetable::Code;
    rt.store.put(Code::Blake2b256, &Block::new(IPLD_RAW, code)).expect("put code")
}

#[test]
fn delegated_storage_persists_across_invocations() {
    let rt = util::construct_and_verify(vec![]);

    // Build two snippets of bytecode:
    // 1) store: SSTORE(0x01, 0x02)
    // 2) load_ret: return 32 bytes stored at slot 0x01
    // EVM bytecode: PUSH1 0x02 PUSH1 0x01 SSTORE STOP
    let store_code: Vec<u8> = vec![0x60, 0x02, 0x60, 0x01, 0x55, 0x00];
    // EVM bytecode: PUSH1 0x01 SLOAD PUSH1 0x00 MSTORE PUSH1 0x20 PUSH1 0x00 RETURN
    let load_code: Vec<u8> = vec![0x60, 0x01, 0x54, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3];
    let store_cid = put_code(&rt, &store_code);
    let load_cid = put_code(&rt, &load_code);

    // Invoke as EOA twice on the same authority address.
    let authority = EthAddress(hex_literal::hex!("00112233445566778899aabbccddeeff00112233"));
    // First call: write storage.
    let p1 = evm::EoaInvokeParams {
        code: store_cid,
        input: vec![],
        caller: authority,   // arbitrary
        receiver: authority, // authority address
        value: TokenAmount::from_whole(0),
    };
    // Immediate caller is self.
    rt.set_caller(*fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID, util::CONTRACT_ID);
    rt.expect_validate_caller_addr(vec![util::CONTRACT_ID]);
    let _ = rt
        .call::<evm::EvmContractActor>(
            evm::Method::InvokeAsEoa as u64,
            IpldBlock::serialize_dag_cbor(&p1).unwrap(),
        )
        .expect("InvokeAsEoa store succeeded");

    // Second call: read storage and return.
    let p2 = evm::EoaInvokeParams {
        code: load_cid,
        input: vec![],
        caller: authority,
        receiver: authority,
        value: TokenAmount::from_whole(0),
    };
    rt.set_caller(*fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID, util::CONTRACT_ID);
    rt.expect_validate_caller_addr(vec![util::CONTRACT_ID]);
    let ret = rt
        .call::<evm::EvmContractActor>(
            evm::Method::InvokeAsEoa as u64,
            IpldBlock::serialize_dag_cbor(&p2).unwrap(),
        )
        .expect("InvokeAsEoa load succeeded")
        .unwrap();

    #[derive(fvm_ipld_encoding::serde::Deserialize)]
    struct InvokeContractReturn {
        output_data: Vec<u8>,
    }
        let out = ret
            .deserialize::<InvokeContractReturn>()
            .map(|x| x.output_data)
            .or_else(|_| {
                fvm_ipld_encoding::from_slice::<InvokeContractReturn>(&ret.data).map(|x| x.output_data)
            })
            .unwrap_or(ret.data);
    // Expect big-endian 0x02 in the last 32 bytes (some runtimes wrap raw output).
    assert!(out.len() >= 32);
    let word = &out[out.len() - 32..];
    assert_eq!(word[31], 0x02);
    for b in &word[..31] {
        assert_eq!(*b, 0x00)
    }
}
