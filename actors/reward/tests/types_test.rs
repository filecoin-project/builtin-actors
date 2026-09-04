// Tests to match:
// - Go: github.com/filecoin-project/go-state-types/builtin/*/reward
// - Solidity: solstice/src/lib/FVMRewards.sol
mod serialization {
    use cid::Cid;
    use fil_actor_reward::{
        AwardBlockRewardParams, CancelPendingParams, ClaimParams, ClaimReturn, ConstructorParams,
        DENOM, DistributionInit, ExplicitDistribution, MAX_RECIPIENTS, PendingWrite,
        PendingWriteOp, RecipientAmount, RecipientShare, RegisterStreamParams,
        RegisterStreamPayload, RemoveStreamParams, SetDistributionParams, SetDistributionPayload,
        SetSharesParams, SetWeightRecordsParams, State, StepWeightRecordsParams, Stream,
        StreamAccrual, StreamsState, ThisEpochRewardReturn, Tombstone, UpdateNetworkKPIParams,
        WeightRecord, WeightRecordUpdate, WeightRecordsPayload,
    };
    use fil_actors_runtime::reward::FilterEstimate;
    use fil_actors_runtime::test_blockstores::MemoryBlockstore;
    use fvm_ipld_encoding::{CborStore, RawBytes, ipld_block::IpldBlock};
    use fvm_shared::address::Address;
    use fvm_shared::bigint::{BigInt, bigint_ser::BigIntDe};
    use fvm_shared::econ::TokenAmount;
    use hex_literal::hex;
    use multihash_codetable::Code;

    fn empty_streams_root() -> Cid {
        MemoryBlockstore::default().put_cbor(&StreamsState::default(), Code::Blake2b256).unwrap()
    }

    fn assert_state_eq(expected: &State, actual: &State) {
        assert_eq!(expected.cumsum_baseline, actual.cumsum_baseline);
        assert_eq!(expected.cumsum_realized, actual.cumsum_realized);
        assert_eq!(expected.effective_network_time, actual.effective_network_time);
        assert_eq!(expected.effective_baseline_power, actual.effective_baseline_power);
        assert_eq!(expected.this_epoch_reward, actual.this_epoch_reward);
        assert_eq!(expected.this_epoch_reward_smoothed, actual.this_epoch_reward_smoothed);
        assert_eq!(expected.this_epoch_baseline_power, actual.this_epoch_baseline_power);
        assert_eq!(expected.epoch, actual.epoch);
        assert_eq!(expected.total_minted_reward, actual.total_minted_reward);
        assert_eq!(expected.total_burn_minted, actual.total_burn_minted);
        assert_eq!(expected.total_explicit_minted, actual.total_explicit_minted);
        assert_eq!(expected.accrued, actual.accrued);
        assert_eq!(expected.swa_timelock_epochs, actual.swa_timelock_epochs);
        assert_eq!(expected.swa_actor, actual.swa_actor);
        assert_eq!(expected.streams_root, actual.streams_root);
    }

    #[test]
    fn constructor_params() {
        let test_cases = vec![
            (ConstructorParams { power: None }, &hex!("f6")[..]),
            // BigInt has no negative zero; zero canonically encodes as an empty byte string.
            (ConstructorParams { power: Some(BigIntDe(BigInt::from(0))) }, &hex!("40")[..]),
            (ConstructorParams { power: Some(BigIntDe(BigInt::from(255))) }, &hex!("4200ff")[..]),
            (ConstructorParams { power: Some(BigIntDe(BigInt::from(256))) }, &hex!("43000100")[..]),
            (ConstructorParams { power: Some(BigIntDe(BigInt::from(-255))) }, &hex!("4201ff")[..]),
            (
                ConstructorParams { power: Some(BigIntDe(BigInt::from(-256))) },
                &hex!("43010100")[..],
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: ConstructorParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn update_network_kpi_params() {
        let test_cases = vec![
            (UpdateNetworkKPIParams { curr_realized_power: None }, &hex!("f6")[..]),
            (
                UpdateNetworkKPIParams { curr_realized_power: Some(BigIntDe(BigInt::from(0))) },
                &hex!("40")[..],
            ),
            (
                UpdateNetworkKPIParams { curr_realized_power: Some(BigIntDe(BigInt::from(255))) },
                &hex!("4200ff")[..],
            ),
            (
                UpdateNetworkKPIParams { curr_realized_power: Some(BigIntDe(BigInt::from(256))) },
                &hex!("43000100")[..],
            ),
            (
                UpdateNetworkKPIParams { curr_realized_power: Some(BigIntDe(BigInt::from(-255))) },
                &hex!("4201ff")[..],
            ),
            (
                UpdateNetworkKPIParams { curr_realized_power: Some(BigIntDe(BigInt::from(-256))) },
                &hex!("43010100")[..],
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: UpdateNetworkKPIParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn award_block_reward_params() {
        let test_cases = vec![
            (
                AwardBlockRewardParams {
                    miner: Address::new_id(100),
                    penalty: TokenAmount::from_atto(0),
                    gas_reward: TokenAmount::from_atto(0),
                    win_count: 0,
                },
                &hex!("84420064404000")[..],
            ),
            (
                AwardBlockRewardParams {
                    miner: delegated_address(),
                    penalty: TokenAmount::from_atto(255),
                    gas_reward: TokenAmount::from_atto(256),
                    win_count: -1,
                },
                &hex!("8456040a11111111111111111111111111111111111111114200ff4300010020")[..],
            ),
            (
                AwardBlockRewardParams {
                    miner: Address::new_id(100),
                    penalty: TokenAmount::from_atto(256),
                    gas_reward: TokenAmount::from_atto(255),
                    win_count: i64::MAX,
                },
                &hex!("84420064430001004200ff1b7fffffffffffffff")[..],
            ),
            (
                AwardBlockRewardParams {
                    miner: delegated_address(),
                    penalty: TokenAmount::from_atto(1),
                    gas_reward: TokenAmount::from_atto(1),
                    win_count: i64::MIN,
                },
                &hex!(
                    "8456040a11111111111111111111111111111111111111114200014200013b7fffffffffffffff"
                )[..],
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: AwardBlockRewardParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn this_epoch_reward_return() {
        let value = ThisEpochRewardReturn {
            this_epoch_reward_smoothed: FilterEstimate::new(BigInt::from(1), BigInt::from(-2)),
            this_epoch_baseline_power: BigInt::from(256),
        };
        let encoded = IpldBlock::serialize_cbor(&value).unwrap().unwrap();
        assert_eq!(
            encoded.data,
            hex!(
                "8282520001000000000000000000000000000000005201020000000000000000000000000000000043000100"
            )
        );
        let decoded: ThisEpochRewardReturn = IpldBlock::deserialize(&encoded).unwrap();
        assert_eq!(value, decoded);
    }

    #[test]
    fn empty_streams_state() {
        let state = StreamsState::default();
        let encoded = IpldBlock::serialize_cbor(&state).unwrap().unwrap();
        assert_eq!(encoded.data, hex!("83808080"));
        let decoded: StreamsState = IpldBlock::deserialize(&encoded).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn populated_streams_state() {
        // Its [0x81, 0x01] pending payload is non-semantic wire data and fails payload validation.
        let state = StreamsState {
            streams: vec![
                Stream { id: 1, weight: WeightRecord::default(), distribution: None },
                Stream {
                    id: 2,
                    weight: WeightRecord { v_start: 5, slope: -2, t_start: 1, floor: 4, cap: 5 },
                    distribution: Some(ExplicitDistribution {
                        writer: Address::new_id(100),
                        shares: vec![RecipientShare { recipient: Address::new_id(101), share: 6 }],
                        payable: vec![RecipientAmount {
                            recipient: Address::new_id(102),
                            amount: TokenAmount::from_atto(7),
                        }]
                        .into(),
                        claimed_period: vec![RecipientAmount {
                            recipient: Address::new_id(103),
                            amount: TokenAmount::from_atto(8),
                        }]
                        .into(),
                    }),
                },
            ],
            tombstones: vec![Tombstone {
                id: 3,
                payable: vec![RecipientAmount {
                    recipient: Address::new_id(104),
                    amount: TokenAmount::from_atto(9),
                }]
                .into(),
            }],
            pending_writes: vec![PendingWrite {
                id: Some(4),
                op: PendingWriteOp::RegisterStream,
                payload: RawBytes::new(vec![0x81, 0x01]),
                effective_epoch: 10,
            }],
        };

        let encoded = IpldBlock::serialize_cbor(&state).unwrap().unwrap();
        assert_eq!(
            encoded.data,
            hex!(
                "83828301850000000000f6830285052101040584420064818242006506818242006642000781824200674200088182038182420068420009818404024281010a"
            )
        );
        let decoded: StreamsState = IpldBlock::deserialize(&encoded).unwrap();
        assert_eq!(state, decoded);
    }

    #[test]
    fn empty_reward_state() {
        let state = State { streams_root: empty_streams_root(), ..Default::default() };
        let encoded = IpldBlock::serialize_cbor(&state).unwrap().unwrap();
        assert_eq!(
            encoded.data,
            hex!(
                "8f404000404082404040004040408000420000d82a5827000171a0e40220d63b11132be58f8f498e5f8c46c4d26b89675b443ff1c47f1e7e3d3cb8d2dcaa"
            )
        );
        let decoded: State = IpldBlock::deserialize(&encoded).unwrap();
        assert_state_eq(&state, &decoded);
    }

    #[test]
    fn populated_reward_state() {
        let state = State {
            cumsum_baseline: BigInt::from(1),
            cumsum_realized: BigInt::from(2),
            effective_network_time: 3,
            effective_baseline_power: BigInt::from(4),
            this_epoch_reward: TokenAmount::from_atto(5),
            this_epoch_reward_smoothed: FilterEstimate::default(),
            this_epoch_baseline_power: BigInt::from(6),
            epoch: 7,
            total_minted_reward: TokenAmount::from_atto(8),
            total_burn_minted: TokenAmount::from_atto(9),
            total_explicit_minted: TokenAmount::from_atto(10),
            accrued: vec![StreamAccrual { id: 2, amount: TokenAmount::from_atto(11) }],
            swa_timelock_epochs: 13,
            swa_actor: Address::new_id(1001),
            streams_root: empty_streams_root(),
        };

        let encoded = IpldBlock::serialize_cbor(&state).unwrap().unwrap();
        assert_eq!(
            encoded.data,
            hex!(
                "8f420001420002034200044200058240404200060742000842000942000a81820242000b0d4300e907d82a5827000171a0e40220d63b11132be58f8f498e5f8c46c4d26b89675b443ff1c47f1e7e3d3cb8d2dcaa"
            )
        );
        let decoded: State = IpldBlock::deserialize(&encoded).unwrap();
        assert_state_eq(&state, &decoded);
    }

    fn weight(v_start: u64, slope: i64, t_start: i64, floor: u64, cap: u64) -> WeightRecord {
        WeightRecord { v_start, slope, t_start, floor, cap }
    }

    fn delegated_address() -> Address {
        Address::new_delegated(
            fil_actors_runtime::EAM_ACTOR_ID,
            &hex!("1111111111111111111111111111111111111111"),
        )
        .unwrap()
    }

    #[test]
    fn set_weight_records_params() {
        let test_cases = vec![
            (
                SetWeightRecordsParams { updates: Vec::new() },
                // [[]] encodes canonically but is rejected by method validation.
                &hex!("8180")[..],
            ),
            (
                SetWeightRecordsParams {
                    updates: vec![
                        WeightRecordUpdate { id: 23, weight: weight(23, -24, 23, 0, 23) },
                        WeightRecordUpdate { id: 24, weight: weight(24, -25, 24, 0, 24) },
                        WeightRecordUpdate { id: 255, weight: weight(255, -256, 255, 0, 255) },
                        WeightRecordUpdate { id: 256, weight: weight(256, -257, 256, 0, 256) },
                        WeightRecordUpdate {
                            id: 65_535,
                            weight: weight(65_535, -65_536, 65_535, 0, 65_535),
                        },
                        WeightRecordUpdate {
                            id: 65_536,
                            weight: weight(65_536, -65_537, 65_536, 0, 65_536),
                        },
                        WeightRecordUpdate {
                            id: u32::MAX as u64,
                            weight: weight(
                                u32::MAX as u64,
                                -(1_i64 << 32),
                                u32::MAX as i64,
                                0,
                                u32::MAX as u64,
                            ),
                        },
                        WeightRecordUpdate {
                            id: 1_u64 << 32,
                            weight: weight(
                                1_u64 << 32,
                                -((1_i64 << 32) + 1),
                                1_i64 << 32,
                                0,
                                1_u64 << 32,
                            ),
                        },
                    ],
                },
                // [[
                //   [23,[23,-24,23,0,23]], [24,[24,-25,24,0,24]],
                //   [255,[255,-256,255,0,255]], [256,[256,-257,256,0,256]],
                //   [65535,[65535,-65536,65535,0,65535]],
                //   [65536,[65536,-65537,65536,0,65536]],
                //   [4294967295,[4294967295,-4294967296,4294967295,0,4294967295]],
                //   [4294967296,[4294967296,-4294967297,4294967296,0,4294967296]]
                // ]]
                &hex!(
                    "81888217851737170017821818851818381818180018188218ff8518ff38ff18ff0018ff"
                    "8219010085190100390100190100001901008219ffff8519ffff39ffff19ffff0019ffff"
                    "821a00010000851a000100003a000100001a00010000001a00010000"
                    "821affffffff851affffffff3affffffff1affffffff001affffffff"
                    "821b0000000100000000851b00000001000000003b0000000100000000"
                    "1b0000000100000000001b0000000100000000"
                )[..],
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: SetWeightRecordsParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn step_weight_records_params() {
        let test_cases = vec![
            (
                StepWeightRecordsParams { updates: Vec::new() },
                // [[]]
                &hex!("8180")[..],
            ),
            (
                StepWeightRecordsParams {
                    updates: vec![WeightRecordUpdate {
                        id: 1_u64 << 32,
                        weight: weight(1_u64 << 32, -((1_i64 << 32) + 1), 65_536, 256, DENOM),
                    }],
                },
                // [[
                //   [4294967296,
                //   [4294967296,-4294967297,65536,256,1000000000000000000]]
                // ]]
                &hex!(
                    "8181821b0000000100000000851b00000001000000003b0000000100000000"
                    "1a000100001901001b0de0b6b3a7640000"
                )[..],
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: StepWeightRecordsParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn weight_records_payload() {
        let test_cases = vec![
            (
                WeightRecordsPayload { updates: Vec::new() },
                // [[]] encodes canonically but is rejected by payload validation.
                &hex!("8180")[..],
            ),
            (
                WeightRecordsPayload {
                    updates: vec![
                        WeightRecordUpdate { id: 24, weight: weight(24, -25, 256, 0, 65_536) },
                        WeightRecordUpdate {
                            id: 1_u64 << 32,
                            weight: weight(1_u64 << 32, -((1_i64 << 32) + 1), 65_536, 256, DENOM),
                        },
                    ],
                },
                // [[
                //   [24,[24,-25,256,0,65536]],
                //   [4294967296,
                //   [4294967296,-4294967297,65536,256,1000000000000000000]]
                // ]]
                &hex!(
                    "81828218188518183818190100001a00010000821b0000000100000000"
                    "851b00000001000000003b00000001000000001a000100001901001b0de0b6b3a7640000"
                )[..],
            ),
        ];

        for (payload, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&payload).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: WeightRecordsPayload = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(payload, decoded);
        }
    }

    #[test]
    fn register_stream_payload() {
        let test_cases = vec![
            (
                RegisterStreamPayload {
                    weight: weight(24, -25, 256, 0, 65_536),
                    distribution: None,
                },
                // [[24,-25,256,0,65536],null]
                &hex!("828518183818190100001a00010000f6")[..],
            ),
            (
                RegisterStreamPayload {
                    weight: weight(1_u64 << 32, -(1_i64 << 32), 65_536, 256, DENOM),
                    distribution: Some(DistributionInit {
                        writer: Address::new_id(1_u64 << 32),
                        shares: vec![RecipientShare {
                            recipient: Address::new_id(101),
                            share: DENOM,
                        }],
                    }),
                },
                // [
                //   [4294967296,-4294967296,65536,256,1000000000000000000],
                //   [byte[008080808010],[[byte[0065],1000000000000000000]]]
                // ]
                &hex!(
                    "82851b00000001000000003affffffff1a000100001901001b0de0b6b3a7640000"
                    "824600808080801081824200651b0de0b6b3a7640000"
                )[..],
            ),
        ];

        for (payload, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&payload).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: RegisterStreamPayload = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(payload, decoded);
        }
    }

    #[test]
    fn set_distribution_payload() {
        let test_cases = vec![
            (
                SetDistributionPayload { writer: Address::new_id(24) },
                // [byte[0018]]
                &hex!("81420018")[..],
            ),
            (
                SetDistributionPayload { writer: Address::new_id(1_u64 << 32) },
                // [byte[008080808010]]
                &hex!("8146008080808010")[..],
            ),
        ];

        for (payload, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&payload).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: SetDistributionPayload = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(payload, decoded);
        }
    }

    #[test]
    fn register_stream_params() {
        let test_cases = vec![
            (
                RegisterStreamParams {
                    id: 24,
                    weight: weight(24, -24, 256, 0, 65_536),
                    distribution: None,
                    activation_epoch: 1_i64 << 32,
                },
                // [24,[24,-24,256,0,65536],null,4294967296]
                &hex!("84181885181837190100001a00010000f61b0000000100000000")[..],
            ),
            (
                RegisterStreamParams {
                    id: 1_u64 << 32,
                    weight: weight(1_u64 << 32, -(1_i64 << 32), 65_536, 256, DENOM),
                    distribution: Some(DistributionInit {
                        writer: delegated_address(),
                        shares: vec![RecipientShare {
                            recipient: Address::new_id(1_u64 << 32),
                            share: DENOM,
                        }],
                    }),
                    activation_epoch: 65_536,
                },
                // [
                //   4294967296,[4294967296,-4294967296,65536,256,1000000000000000000],
                //   [byte[040a1111111111111111111111111111111111111111],
                //     [[byte[008080808010],1000000000000000000]]],65536
                // ]
                &hex!(
                    "841b0000000100000000851b00000001000000003affffffff1a00010000190100"
                    "1b0de0b6b3a76400008256040a1111111111111111111111111111111111111111"
                    "8182460080808080101b0de0b6b3a76400001a00010000"
                )[..],
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: RegisterStreamParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn remove_stream_params() {
        let test_cases = vec![
            (
                RemoveStreamParams { id: 24 },
                // [24]
                &hex!("811818")[..],
            ),
            (
                RemoveStreamParams { id: 256 },
                // [256]
                &hex!("81190100")[..],
            ),
            (
                RemoveStreamParams { id: 65_536 },
                // [65536]
                &hex!("811a00010000")[..],
            ),
            (
                RemoveStreamParams { id: 1_u64 << 32 },
                // [4294967296]
                &hex!("811b0000000100000000")[..],
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: RemoveStreamParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn set_distribution_params() {
        let test_cases = vec![
            (
                SetDistributionParams { id: 24, writer: Address::new_id(1_u64 << 32) },
                // [24,byte[008080808010]]
                &hex!("82181846008080808010")[..],
            ),
            (
                SetDistributionParams { id: 256, writer: delegated_address() },
                // [256,byte[040a1111111111111111111111111111111111111111]]
                &hex!("8219010056040a1111111111111111111111111111111111111111")[..],
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: SetDistributionParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn set_shares_params() {
        let max_shares = (0..MAX_RECIPIENTS)
            .map(|index| RecipientShare {
                recipient: Address::new_id(1000 + index as u64),
                share: DENOM / MAX_RECIPIENTS as u64,
            })
            .collect();
        let test_cases = vec![
            (
                SetSharesParams { id: 24, shares: Vec::new() },
                // [24,[]]
                &hex!("82181880")[..],
            ),
            (
                SetSharesParams {
                    id: 256,
                    shares: vec![
                        RecipientShare { recipient: Address::new_id(24), share: 24 },
                        RecipientShare { recipient: Address::new_id(256), share: 256 },
                        RecipientShare { recipient: Address::new_id(65_536), share: 65_536 },
                        RecipientShare {
                            recipient: Address::new_id(1_u64 << 32),
                            share: 1_u64 << 32,
                        },
                    ],
                },
                // [
                //   256,[[byte[0018],24],[byte[008002],256],
                //   [byte[00808004],65536],[byte[008080808010],4294967296]]
                // ]
                &hex!(
                    "821901008482420018181882430080021901008244008080041a00010000"
                    "82460080808080101b0000000100000000"
                )[..],
            ),
            (
                SetSharesParams { id: 65_536, shares: max_shares },
                // [65536,64 * [byte[f01000..f01063],15625000000000000]]
                &hex!(
                    "821a000100009840824300e8071b003782dace9d9000824300e9071b003782dace9d9000"
                    "824300ea071b003782dace9d9000824300eb071b003782dace9d9000824300ec071b0037"
                    "82dace9d9000824300ed071b003782dace9d9000824300ee071b003782dace9d90008243"
                    "00ef071b003782dace9d9000824300f0071b003782dace9d9000824300f1071b003782da"
                    "ce9d9000824300f2071b003782dace9d9000824300f3071b003782dace9d9000824300f4"
                    "071b003782dace9d9000824300f5071b003782dace9d9000824300f6071b003782dace9d"
                    "9000824300f7071b003782dace9d9000824300f8071b003782dace9d9000824300f9071b"
                    "003782dace9d9000824300fa071b003782dace9d9000824300fb071b003782dace9d9000"
                    "824300fc071b003782dace9d9000824300fd071b003782dace9d9000824300fe071b0037"
                    "82dace9d9000824300ff071b003782dace9d900082430080081b003782dace9d90008243"
                    "0081081b003782dace9d900082430082081b003782dace9d900082430083081b003782da"
                    "ce9d900082430084081b003782dace9d900082430085081b003782dace9d900082430086"
                    "081b003782dace9d900082430087081b003782dace9d900082430088081b003782dace9d"
                    "900082430089081b003782dace9d90008243008a081b003782dace9d90008243008b081b"
                    "003782dace9d90008243008c081b003782dace9d90008243008d081b003782dace9d9000"
                    "8243008e081b003782dace9d90008243008f081b003782dace9d900082430090081b0037"
                    "82dace9d900082430091081b003782dace9d900082430092081b003782dace9d90008243"
                    "0093081b003782dace9d900082430094081b003782dace9d900082430095081b003782da"
                    "ce9d900082430096081b003782dace9d900082430097081b003782dace9d900082430098"
                    "081b003782dace9d900082430099081b003782dace9d90008243009a081b003782dace9d"
                    "90008243009b081b003782dace9d90008243009c081b003782dace9d90008243009d081b"
                    "003782dace9d90008243009e081b003782dace9d90008243009f081b003782dace9d9000"
                    "824300a0081b003782dace9d9000824300a1081b003782dace9d9000824300a2081b0037"
                    "82dace9d9000824300a3081b003782dace9d9000824300a4081b003782dace9d90008243"
                    "00a5081b003782dace9d9000824300a6081b003782dace9d9000824300a7081b003782da"
                    "ce9d9000"
                )[..],
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: SetSharesParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn cancel_pending_params() {
        let test_cases = vec![
            (
                CancelPendingParams { id: None, op: PendingWriteOp::SetWeightRecords },
                // [null,0]
                &hex!("82f600")[..],
            ),
            (
                CancelPendingParams { id: None, op: PendingWriteOp::StepWeightRecords },
                // [null,1]
                &hex!("82f601")[..],
            ),
            (
                CancelPendingParams { id: Some(24), op: PendingWriteOp::RegisterStream },
                // [24,2]
                &hex!("82181802")[..],
            ),
            (
                CancelPendingParams { id: Some(256), op: PendingWriteOp::RemoveStream },
                // [256,3]
                &hex!("8219010003")[..],
            ),
            (
                CancelPendingParams { id: Some(65_536), op: PendingWriteOp::SetDistribution },
                // [65536,4]
                &hex!("821a0001000004")[..],
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: CancelPendingParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn claim_params() {
        let max_wallets =
            (1000..1000 + MAX_RECIPIENTS as u64).map(Address::new_id).collect::<Vec<_>>();
        let test_cases = vec![
            (
                ClaimParams { id: 1_u64 << 32, wallets: Vec::new() },
                // [4294967296,[]]
                &hex!("821b000000010000000080")[..],
            ),
            (
                ClaimParams {
                    id: 65_536,
                    wallets: vec![Address::new_id(1_u64 << 32), delegated_address()],
                },
                // [
                //   65536,[byte[008080808010],
                //   byte[040a1111111111111111111111111111111111111111]]
                // ]
                &hex!(
                    "821a000100008246008080808010"
                    "56040a1111111111111111111111111111111111111111"
                )[..],
            ),
            (
                ClaimParams { id: 65_536, wallets: max_wallets },
                &hex!(
                    "821a0001000098404300e8074300e9074300ea074300eb074300ec074300ed074300ee0743"
                    "00ef074300f0074300f1074300f2074300f3074300f4074300f5074300f6074300f7074300"
                    "f8074300f9074300fa074300fb074300fc074300fd074300fe074300ff0743008008430081"
                    "08430082084300830843008408430085084300860843008708430088084300890843008a08"
                    "43008b0843008c0843008d0843008e0843008f084300900843009108430092084300930843"
                    "009408430095084300960843009708430098084300990843009a0843009b0843009c084300"
                    "9d0843009e0843009f084300a0084300a1084300a2084300a3084300a4084300a5084300a6"
                    "084300a708"
                )[..],
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: ClaimParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn claim_return() {
        let test_cases = vec![
            (
                ClaimReturn { amounts: Vec::new() },
                // [[]]
                &hex!("8180")[..],
            ),
            (
                ClaimReturn {
                    amounts: [0, 24, 256, 65_536, 1_i64 << 32]
                        .into_iter()
                        .map(TokenAmount::from_atto)
                        .collect(),
                },
                // [[byte[],byte[0018],byte[000100],byte[00010000],byte[000100000000]]]
                &hex!("81854042001843000100440001000046000100000000")[..],
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected_hex);
            let decoded: ClaimReturn = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }
}
