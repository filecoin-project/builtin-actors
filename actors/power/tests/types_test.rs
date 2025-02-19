// Tests to match with Go github.com/filecoin-project/go-state-types/builtin/*/power
mod serialization {
    use hex_literal::hex;

    use fil_actor_power::CurrentTotalPowerReturn;
    use fvm_ipld_encoding::ipld_block::IpldBlock;

    use fil_actors_runtime::reward::FilterEstimate;
    use fvm_shared::bigint::BigInt;
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::sector::StoragePower;

    #[test]
    fn current_total_power_return() {
        let test_cases = vec![
            (
                CurrentTotalPowerReturn {
                    raw_byte_power: Default::default(),
                    quality_adj_power: Default::default(),
                    pledge_collateral: Default::default(),
                    quality_adj_power_smoothed: Default::default(),
                    ramp_start_epoch: Default::default(),
                    ramp_duration_epochs: Default::default(),
                },
                // [byte[],byte[],byte[],[byte[],byte[]],0,0]
                &hex!("864040408240400000")[..],
            ),
            (
                CurrentTotalPowerReturn {
                    raw_byte_power: StoragePower::from(1 << 20),
                    quality_adj_power: StoragePower::from(1 << 21),
                    pledge_collateral: TokenAmount::from_atto(1 << 22),
                    quality_adj_power_smoothed: FilterEstimate::new(BigInt::from(1 << 23), BigInt::from(1 << 24)),
                    ramp_start_epoch: 25,
                    ramp_duration_epochs: 26,
                },
                // FilterEstimate BigInts have a precision shift of 128, so they end up larger than the others.
                // [byte[00100000],byte[00200000],byte[00400000],[byte[0080000000000000000000000000000000000000],byte[000100000000000000000000000000000000000000]],25,26]
                &hex!("8644001000004400200000440040000082540080000000000000000000000000000000000000550001000000000000000000000000000000000000001819181a"),
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: CurrentTotalPowerReturn = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }
}
