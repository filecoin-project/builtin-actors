use fil_actor_ethaccount as ethaccount;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::DAG_CBOR;
use fvm_shared::error::ExitCode;

mod util;

// A small, deterministic set of malformed CBOR payloads for ApplyAndCall to ensure robust
// rejection and no panics on deserialization or validation.
#[test]
fn apply_and_call_rejects_malformed_cbor() {
    // We don't need full EthAccount setup because decode fails before actor code executes.
    let mut rt = util::new_runtime();

    // Helper to call ApplyAndCall with raw CBOR bytes and assert error.
    let try_call = |rt: &mut fil_actors_runtime::test_utils::MockRuntime, cbor: Vec<u8>| {
        let res = rt.call::<ethaccount::EthAccountActor>(
            ethaccount::Method::ApplyAndCall as u64,
            Some(IpldBlock { codec: DAG_CBOR, data: cbor }),
        );
        assert!(res.is_err(), "malformed CBOR should be rejected");
        // Any non-OK exit is fine; USR_SERIALIZATION or USR_ILLEGAL_ARGUMENT are acceptable.
        let code = res.err().unwrap().exit_code();
        assert!(code != ExitCode::OK);
    };

    // 1) Not an array at top-level: UnsignedInt(7)
    {
        let buf = vec![0x07];
        try_call(&mut rt, buf);
    }

    // 2) Array(1) instead of Array(2): [ [] ]
    {
        let buf = vec![0x81, 0x80];
        try_call(&mut rt, buf);
    }

    // 3) Array(2) but first element not an array: [ 1, [to,value,input] ]
    {
        let mut buf = vec![
            0x82, // array(2)
            0x01, // unsigned 1
            0x83, // array(3) call tuple
            0x54, // bytes(20)
        ];
        buf.extend_from_slice(&[0u8; 20]);
        buf.extend_from_slice(&[0x41, 0x00]); // bytes(1)=0
        buf.push(0x40); // bytes(0)
        try_call(&mut rt, buf);
    }

    // 4) Array(2), inner list with wrong tuple arity: [ [ tuple(5) ], call ]
    {
        let mut buf = vec![
            0x82, // array(2)
            0x81, // array(1)
            0x85, // array(5)
            0x83, // array(3) call tuple
            0x54, // bytes(20)
        ];
        buf.extend_from_slice(&[0u8; 20]);
        buf.extend_from_slice(&[0x41, 0x00]); // bytes(1)=0
        buf.push(0x40); // bytes(0)
        try_call(&mut rt, buf);
    }
}

