use cid::Cid;
use fil_actor_evm as evm;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::test_utils::MockRuntime;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::{IPLD_RAW, address::Address as FilAddress, econ::TokenAmount, error::ExitCode, sys::SendFlags};

mod util;

// Depth-limit positive test (ApplyAndCall-driven):
// - First ApplyAndCall sets mapping C->D (no sends).
// - Second ApplyAndCall sets A->B and calls A; InvokeAsEoa enters authority context and executes B.
//   B's bytecode performs CALL to EOA C. Since we are in authority context, mapping C->D must NOT be followed.
//   Expect exactly two sends overall: GetBytecode(delegate=B) and InvokeAsEoa; no further sends.
//   ApplyAndCallReturn.status == 1.
#[test]
fn apply_and_call_depth_limit_invokeaseoa() {
    // Build delegate bytecode that CALLs to EOA C with zero value and no data, then STOP.
    // EVM stack for CALL: gas, to, value, in_off, in_sz, out_off, out_sz
    // Program: PUSH1 0 PUSH1 0 PUSH1 0 PUSH1 0 PUSH1 0 PUSH20 <C> PUSH4 0x000F4240 CALL STOP

    // Choose two fixed public keys for authority recovery used in the two calls.
    // pk_c derives EthAddress C; pk_a derives EthAddress A.
    let mut pk_c = [0u8; 65];
    pk_c[0] = 0x04; for i in 1..65 { pk_c[i] = 0xC1; }
    let mut pk_a = [0u8; 65];
    pk_a[0] = 0x04; for i in 1..65 { pk_a[i] = 0xA1; }

    // Derive EthAddress from uncompressed pubkey as in actor: keccak(pubkey[1:])[12:]
    use fil_actors_runtime::test_utils::hash as rt_hash;
    use fvm_shared::crypto::hash::SupportedHashes;
    let (keccak_c, _) = rt_hash(SupportedHashes::Keccak256, &pk_c[1..]);
    let (keccak_a, _) = rt_hash(SupportedHashes::Keccak256, &pk_a[1..]);
    let mut c20 = [0u8; 20]; c20.copy_from_slice(&keccak_c[12..32]);
    let mut a20 = [0u8; 20]; a20.copy_from_slice(&keccak_a[12..32]);
    let c_eth = EthAddress(c20);
    let a_eth = EthAddress(a20);

    // Construct bytecode for delegate B that CALLs C.
    let mut bytecode: Vec<u8> = Vec::new();
    // PUSH1 0 (out_size, out_off, in_size, in_off, value)
    for _ in 0..5 { bytecode.extend_from_slice(&[0x60, 0x00]); }
    // PUSH20 C address
    bytecode.push(0x73); bytecode.extend_from_slice(c_eth.as_ref());
    // PUSH4 0x000F4240 (1_000_000 gas)
    bytecode.extend_from_slice(&[0x63, 0x00, 0x0F, 0x42, 0x40]);
    // CALL, STOP
    bytecode.push(0xF1); bytecode.push(0x00);

    // Initialize EVM actor with the above bytecode as its contract code.
    let mut rt = util::construct_and_verify(bytecode.clone());
    // Tolerate any early gas_available checks in the call path.
    for _ in 0..10 { rt.expect_gas_available(10_000_000_000); }

    // Prepare an EVM actor code CID to return from GetBytecode.
    let bytecode_cid = Cid::try_from("baeaikaia").unwrap();
    rt.store.put_keyed(&bytecode_cid, bytecode.as_slice()).unwrap();

    // First ApplyAndCall: set mapping C -> D (D arbitrary); ensure no sends by calling a different EOA.
    {
        // Recover always returns pk_c so authority == C.
        rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_c));
        let d_eth = EthAddress::from_id(101); // arbitrary delegate; mapping should not be followed in this phase
        let list = vec![evm::DelegationParam { chain_id: 0, address: d_eth, nonce: 0, y_parity: 0, r: vec![1u8;32], s: vec![1u8;32] }];
        // Call to some other EOA (not C) to avoid following mapping and avoid sends.
        let to_other = EthAddress::from_id(202);
        let params = evm::ApplyAndCallParams { list, call: evm::ApplyCall { to: to_other, value: vec![], input: vec![] } };
        for _ in 0..10 { rt.expect_gas_available(10_000_000_000); }
        rt.expect_validate_caller_any();
        let res = rt.call::<evm::EvmContractActor>(evm::Method::ApplyAndCall as u64, IpldBlock::serialize_dag_cbor(&params).unwrap());
        assert!(res.is_ok());
        // No sends expected in this phase; verify and reset expectations.
        rt.verify();
        rt.reset();
    }

    // Second ApplyAndCall: set mapping A -> B (B is receiver EVM actor) and call A; should follow A->B and execute B in authority context.
    {
        // Recover always returns pk_a so authority == A.
        rt.recover_secp_pubkey_fn = Box::new(move |_, _| Ok(pk_a));

        // Delegate B is the receiver EVM actor (ID 0) with known f4 address set by util::construct_and_verify.
        // Build mapping tuple for A -> B (B as EthAddress derived from receiver's f4).
        let _b_f4 = FilAddress::new_delegated(fil_actors_runtime::EAM_ACTOR_ID, &util::CONTRACT_ADDRESS).unwrap();
        let b_eth: EthAddress = EthAddress(util::CONTRACT_ADDRESS);
        let list = vec![evm::DelegationParam { chain_id: 0, address: b_eth, nonce: 0, y_parity: 0, r: vec![2u8;32], s: vec![2u8;32] }];
        let params = evm::ApplyAndCallParams { list, call: evm::ApplyCall { to: a_eth, value: vec![], input: vec![] } };

        // Expect exactly two sends: GetBytecode(delegate=B) and InvokeAsEoa (any params).
        rt.expect_send(
            FilAddress::new_id(0),
            evm::Method::GetBytecode as u64,
            None,
            TokenAmount::from_whole(0),
            None,
            SendFlags::READ_ONLY,
            IpldBlock::serialize_cbor(&bytecode_cid).unwrap(),
            ExitCode::OK,
            None,
        );
        rt.expect_send_any_params(
            rt.receiver,
            evm::Method::InvokeAsEoa as u64,
            TokenAmount::from_whole(0),
            None,
            SendFlags::default(),
            Some(IpldBlock { codec: IPLD_RAW, data: Vec::new() }),
            ExitCode::OK,
            None,
        );

        rt.expect_validate_caller_any();
        let res = rt.call::<evm::EvmContractActor>(evm::Method::ApplyAndCall as u64, IpldBlock::serialize_dag_cbor(&params).unwrap());
        assert!(res.is_ok());
        let blk = res.unwrap().unwrap();
        // Decode ApplyAndCallReturn [status, output_data]
        #[derive(fvm_ipld_encoding::serde::Deserialize)]
        struct ApplyAndCallReturn(u64, Vec<u8>);
        let r: ApplyAndCallReturn = blk.deserialize().expect("decode ApplyAndCallReturn");
        assert_eq!(r.0, 1, "expected success status from ApplyAndCall");

        // Verify no unexpected sends occurred.
        rt.verify();
    }
}
