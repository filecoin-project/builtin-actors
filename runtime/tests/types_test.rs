// Tests to match with Go github.com/filecoin-project/go-state-types/*/BatchReturn
mod serialization {
    use fil_actors_runtime::{BatchReturn, BatchReturnGen};
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::error::ExitCode;

    #[test]
    fn batch_return() {
        let mut test_cases = vec![];

        let mut gen = BatchReturnGen::new(0);
        test_cases.push((
            gen.gen(),
            // [0,[]]
            "820080",
        ));

        gen = BatchReturnGen::new(1);
        gen.add_success();
        test_cases.push((
            gen.gen(),
            // [1,[]]
            "820180",
        ));

        gen = BatchReturnGen::new(1);
        gen.add_fail(ExitCode::USR_ILLEGAL_ARGUMENT);
        test_cases.push((
            gen.gen(),
            // [0,[[0,16]]]
            "820081820010",
        ));

        gen = BatchReturnGen::new(5);
        gen.add_success();
        gen.add_fail(ExitCode::SYS_OUT_OF_GAS);
        gen.add_fail(ExitCode::USR_ILLEGAL_STATE);
        gen.add_success();
        gen.add_fail(ExitCode::USR_ILLEGAL_ARGUMENT);

        test_cases.push((
            gen.gen(),
            // [2,[[1,7],[2,20],[4,16]]]
            "820283820107820214820410",
        ));

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(const_hex::encode(&encoded.data), expected_hex);
            let decoded: BatchReturn = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }
}
