// Tests to match with Go github.com/filecoin-project/go-state-types/*/BatchReturn
mod serialization {
    use hex_literal::hex;

    use fil_actors_runtime::{BatchReturn, BatchReturnGen};
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::error::ExitCode;

    #[test]
    fn batch_return() {
        let mut test_cases = vec![];

        let mut batch = BatchReturnGen::new(0);
        test_cases.push((
            batch.generate(),
            // [0,[]]
            &hex!("820080")[..],
        ));

        batch = BatchReturnGen::new(1);
        batch.add_success();
        test_cases.push((
            batch.generate(),
            // [1,[]]
            &hex!("820180"),
        ));

        batch = BatchReturnGen::new(1);
        batch.add_fail(ExitCode::USR_ILLEGAL_ARGUMENT);
        test_cases.push((
            batch.generate(),
            // [0,[[0,16]]]
            &hex!("820081820010"),
        ));

        batch = BatchReturnGen::new(5);
        batch.add_success();
        batch.add_fail(ExitCode::SYS_OUT_OF_GAS);
        batch.add_fail(ExitCode::USR_ILLEGAL_STATE);
        batch.add_success();
        batch.add_fail(ExitCode::USR_ILLEGAL_ARGUMENT);

        test_cases.push((
            batch.generate(),
            // [2,[[1,7],[2,20],[4,16]]]
            &hex!("820283820107820214820410"),
        ));

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: BatchReturn = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }
}
