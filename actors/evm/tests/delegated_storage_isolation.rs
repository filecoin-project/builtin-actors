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

// Storage persistence across different delegated bytecode for the same authority.
// Simulates switching delegates (A->B then A->C) by changing the bytecode executed under
// InvokeAsEoa while keeping the authority constant. Storage must persist under the authority.
#[test]
fn delegated_storage_persists_across_delegate_changes() {
    let rt = util::construct_and_verify(vec![]);

    // Code 1: store 0x02 at slot 0x01
    let store_code: Vec<u8> = vec![0x60, 0x02, 0x60, 0x01, 0x55, 0x00];
    // Code 2: load slot 0x01 and return the 32-byte word
    let load_code: Vec<u8> = vec![0x60, 0x01, 0x54, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xf3];
    let store_cid = put_code(&rt, &store_code);
    let load_cid = put_code(&rt, &load_code);

    // Authority address
    let authority = EthAddress(hex_literal::hex!("11223344556677889900aabbccddeeff00112233"));

    // 1) Execute store under Authority A
    let p1 = evm::EoaInvokeParams {
        code: store_cid,
        input: vec![],
        caller: authority,
        receiver: authority,
        value: TokenAmount::from_whole(0),
    };
    // Immediate caller is this actor (internal trampoline)
    rt.set_caller(*fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID, util::CONTRACT_ID);
    rt.expect_validate_caller_addr(vec![util::CONTRACT_ID]);
    let _ = rt
        .call::<evm::EvmContractActor>(
            evm::Method::InvokeAsEoa as u64,
            IpldBlock::serialize_dag_cbor(&p1).unwrap(),
        )
        .expect("InvokeAsEoa store succeeded");

    // 2) Execute load with different bytecode (simulating a different delegate "C")
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
    assert!(out.len() >= 32);
    let word = &out[out.len() - 32..];
    assert_eq!(word[31], 0x02); // lower byte is 0x02
    for b in &word[..31] {
        assert_eq!(*b, 0x00)
    }

    // 3) "Clear" and re-delegate is orthogonal to storage: running the load bytecode again must
    // still observe the same stored value under the same authority.
    rt.set_caller(*fil_actors_runtime::test_utils::EVM_ACTOR_CODE_ID, util::CONTRACT_ID);
    rt.expect_validate_caller_addr(vec![util::CONTRACT_ID]);
    let ret2 = rt
        .call::<evm::EvmContractActor>(
            evm::Method::InvokeAsEoa as u64,
            IpldBlock::serialize_dag_cbor(&p2).unwrap(),
        )
        .expect("InvokeAsEoa second load succeeded")
        .unwrap();
    let out2 =
        ret2.deserialize::<InvokeContractReturn>().map(|x| x.output_data).unwrap_or(ret2.data);
    assert!(out2.len() >= 32);
    let word2 = &out2[out2.len() - 32..];
    assert_eq!(word2[31], 0x02);
}
