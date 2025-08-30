// Tests to match with Go github.com/filecoin-project/go-state-types/builtin/*/miner
mod serialization {
    use std::iter;
    use std::ops::Range;
    use std::str::FromStr;

    use cid::Cid;
    use hex_literal::hex;

    use fil_actor_miner::{
        Deadline, ExpirationSet, PowerPair, ProveCommitSectorsNIParams, SectorNIActivationInfo,
        SectorOnChainInfo, SectorOnChainInfoFlags,
    };
    use fvm_ipld_bitfield::BitField;
    use fvm_ipld_bitfield::iter::Ranges;
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::bigint::BigInt;
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::sector::{RegisteredAggregateProof, RegisteredSealProof};
    use num_traits::Zero;

    #[test]
    fn prove_commit_sectors_ni_params() {
        let test_cases = vec![
            (
                ProveCommitSectorsNIParams {
                    sectors: vec![],
                    aggregate_proof: vec![].into(),
                    seal_proof_type: RegisteredSealProof::StackedDRG32GiBV1P1,
                    aggregate_proof_type: RegisteredAggregateProof::SnarkPackV2,
                    proving_deadline: 2,
                    require_activation_success: false,

                    sealer_id_actor: None,
                    sealer_id_verifier_signature: None,
                    final_sector_numbers: None,
                },
                // [[],byte[],8,1,2,false,null,null,null]
                &hex!("898040080102f4f6f6f6")[..],
            ),
            (
                ProveCommitSectorsNIParams {
                    sectors: vec![SectorNIActivationInfo {
                        sealing_number: 1,
                        sealer_id: 2,
                        sealed_cid: Cid::from_str("bagboea4seaaqa").unwrap(),
                        sector_number: 3,
                        seal_rand_epoch: 4,
                        expiration: 5,
                    }],
                    seal_proof_type: RegisteredSealProof::StackedDRG32GiBV1P2_Feat_NiPoRep,
                    aggregate_proof: vec![0xde, 0xad, 0xbe, 0xef].into(),
                    aggregate_proof_type: RegisteredAggregateProof::SnarkPackV2,
                    proving_deadline: 6,
                    require_activation_success: true,

                    sealer_id_actor: None,
                    sealer_id_verifier_signature: None,
                    final_sector_numbers: None,
                },
                // [[[1,2,bagboea4seaaqa,3,4,5]],byte[deadbeef],18,1,6,true,null,null,null]
                &hex!("8981860102d82a49000182e2039220010003040544deadbeef120106f5f6f6f6"),
            ),
            (
                ProveCommitSectorsNIParams {
                    sectors: vec![
                        SectorNIActivationInfo {
                            sealing_number: 1,
                            sealer_id: 2,
                            sealed_cid: Cid::from_str("bagboea4seaaqa").unwrap(),
                            sector_number: 3,
                            seal_rand_epoch: 4,
                            expiration: 5,
                        },
                        SectorNIActivationInfo {
                            sealing_number: 6,
                            sealer_id: 7,
                            sealed_cid: Cid::from_str("bagboea4seaaqc").unwrap(),
                            sector_number: 8,
                            seal_rand_epoch: 9,
                            expiration: 10,
                        },
                    ],
                    seal_proof_type: RegisteredSealProof::StackedDRG32GiBV1P2_Feat_NiPoRep,
                    aggregate_proof: vec![0xde, 0xad, 0xbe, 0xef].into(),
                    aggregate_proof_type: RegisteredAggregateProof::SnarkPackV2,
                    proving_deadline: 11,
                    require_activation_success: false,

                    sealer_id_actor: None,
                    sealer_id_verifier_signature: None,
                    final_sector_numbers: None,
                },
                // [[[1,2,bagboea4seaaqa,3,4,5],[6,7,bagboea4seaaqc,8,9,10]],byte[deadbeef],18,1,11,false,null,null,null]
                &hex!(
                    "8982860102d82a49000182e20392200100030405860607d82a49000182e2039220010108090a44deadbeef12010bf4f6f6f6"
                ),
            ),
        ];

        for (params, expected) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected);
            let decoded: ProveCommitSectorsNIParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn sector_on_chain_info() {
        let test_cases = vec![
            (
                SectorOnChainInfo { ..Default::default() },
                // [0,-1,{"/":"baeaaaaa"},[],0,0,[],[],[],null,null,0,null,null,0,[]]
                &hex!("900020d82a450001000000800000404040f6f600f6f60040")[..],
                // same on write as read
                &hex!("900020d82a450001000000800000404040f6f600f6f60040")[..],
            ),
            (
                SectorOnChainInfo {
                    sector_number: 1,
                    seal_proof: RegisteredSealProof::StackedDRG32GiBV1P1,
                    sealed_cid: Cid::from_str("bagboea4seaaqa").unwrap(),
                    deprecated_deal_ids: vec![],
                    activation: 2,
                    expiration: 3,
                    deal_weight: 4.into(),
                    verified_deal_weight: 5.into(),
                    initial_pledge: TokenAmount::from_whole(6),
                    expected_day_reward: None,
                    expected_storage_pledge: None,
                    power_base_epoch: 9,
                    replaced_day_reward: None,
                    sector_key_cid: None,
                    flags: Default::default(),
                    daily_fee: TokenAmount::from_whole(11),
                },
                // '[1,8,{"/":"bagboea4seaaqa"},[],2,3,[AAQ],[AAU],[AFNESDXsWAAA],null,null,9,null,null,0,[AJin2bgxTAAA]]'
                &hex!(
                    "900108d82a49000182e20392200100800203420004420005490053444835ec580000f6f609f6f600490098a7d9b8314c0000"
                ),
                // same on write as read
                &hex!(
                    "900108d82a49000182e20392200100800203420004420005490053444835ec580000f6f609f6f600490098a7d9b8314c0000"
                ),
            ),
            (
                SectorOnChainInfo {
                    sector_number: 1,
                    seal_proof: RegisteredSealProof::StackedDRG32GiBV1P1,
                    sealed_cid: Cid::from_str("bagboea4seaaqa").unwrap(),
                    deprecated_deal_ids: vec![],
                    activation: 2,
                    expiration: 3,
                    deal_weight: 4.into(),
                    verified_deal_weight: 5.into(),
                    initial_pledge: TokenAmount::from_whole(6),
                    expected_day_reward: None,
                    expected_storage_pledge: None,
                    power_base_epoch: 9,
                    replaced_day_reward: None,
                    sector_key_cid: Some(Cid::from_str("baga6ea4seaaqc").unwrap()),
                    flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER,
                    daily_fee: TokenAmount::from_whole(11),
                },
                // [1,8,{"/":"bagboea4seaaqa"},[],2,3,[AAQ],[AAU],[AFNESDXsWAAA],null,null,9,null,{"/":"baga6ea4seaaqc"},1,[AJin2bgxTAAA]]
                &hex!(
                    "900108d82a49000182e20392200100800203420004420005490053444835ec580000f6f609f6d82a49000181e2039220010101490098a7d9b8314c0000"
                ),
                // same on write as read
                &hex!(
                    "900108d82a49000182e20392200100800203420004420005490053444835ec580000f6f609f6d82a49000181e2039220010101490098a7d9b8314c0000"
                ),
            ),
            (
                // old format stored on chain but materialised as the new format with a default value at the end
                SectorOnChainInfo {
                    sector_number: 1,
                    seal_proof: RegisteredSealProof::StackedDRG64GiBV1P1,
                    sealed_cid: Cid::from_str("bagboea4seaaqa").unwrap(),
                    deprecated_deal_ids: vec![],
                    activation: 2,
                    expiration: 3,
                    deal_weight: 4.into(),
                    verified_deal_weight: 5.into(),
                    initial_pledge: TokenAmount::from_whole(6),
                    expected_day_reward: None,
                    expected_storage_pledge: None,
                    power_base_epoch: 9,
                    replaced_day_reward: None,
                    sector_key_cid: None,
                    flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER,
                    daily_fee: TokenAmount::zero(), // default, not present in the binary
                },
                // [1,9,{"/":"bagboea4seaaqa"},[],2,3,[AAQ],[AAU],[AFNESDXsWAAA],null,null,9,null,null,1]
                &hex!(
                    "8f0109d82a49000182e20392200100800203420004420005490053444835ec580000f6f609f6f601"
                ),
                // extra field at the end on write, zero BigInt (bytes) for daily_fee
                // [1,9,{"/":"bagboea4seaaqa"},[],2,3,[AAQ],[AAU],[AFNESDXsWAAA],null,null,9,null,null,1,[]]
                &hex!(
                    "900109d82a49000182e20392200100800203420004420005490053444835ec580000f6f609f6f60140"
                ),
            ),
        ];

        for (idx, (params, read_bytes, write_bytes)) in test_cases.into_iter().enumerate() {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, write_bytes, "Test case {} encoding failed", idx);

            let decoded: SectorOnChainInfo =
                IpldBlock::deserialize(&IpldBlock { codec: 0x71, data: read_bytes.to_vec() })
                    .unwrap();
            assert_eq!(params, decoded, "Test case {} decoding failed", idx);
        }
    }

    #[test]
    fn expiration_set() {
        // ExpirationSet's fields are all bytes or byte tuples
        let test_cases = vec![
            (
                ExpirationSet { ..Default::default() },
                // [[],[],[],[[],[]],[[],[]],[]]
                &hex!("8640404082404082404040")[..],
                // same on write as read
                &hex!("8640404082404082404040")[..],
            ),
            (
                ExpirationSet {
                    on_time_sectors: BitField::from_ranges(Ranges::new(
                        iter::once(0..1).collect::<Vec<Range<u64>>>(),
                    )),
                    early_sectors: BitField::from_ranges(Ranges::new(
                        iter::once(1..2).collect::<Vec<Range<u64>>>(),
                    )),
                    on_time_pledge: TokenAmount::from_whole(2),
                    active_power: PowerPair::new(BigInt::from(3), BigInt::from(4)),
                    faulty_power: PowerPair::new(BigInt::from(5), BigInt::from(6)),
                    fee_deduction: TokenAmount::from_whole(7),
                },
                // [[DA],[GA],[ABvBbWdOyAAA],[[AAM],[AAQ]],[[AAU],[AAY]],[AGEk/umTvAAA]]
                &hex!(
                    "86410c411849001bc16d674ec80000824200034200048242000542000649006124fee993bc0000"
                ),
                // same on write as read
                &hex!(
                    "86410c411849001bc16d674ec80000824200034200048242000542000649006124fee993bc0000"
                ),
            ),
            (
                ExpirationSet {
                    on_time_sectors: BitField::from_ranges(Ranges::new(
                        iter::once(0..1).collect::<Vec<Range<u64>>>(),
                    )),
                    early_sectors: BitField::from_ranges(Ranges::new(
                        iter::once(1..2).collect::<Vec<Range<u64>>>(),
                    )),
                    on_time_pledge: TokenAmount::from_whole(2),
                    active_power: PowerPair::new(BigInt::from(3), BigInt::from(4)),
                    faulty_power: PowerPair::new(BigInt::from(5), BigInt::from(6)),
                    fee_deduction: TokenAmount::zero(),
                },
                // [[DA],[GA],[ABvBbWdOyAAA],[[AAM],[AAQ]],[[AAU],[AAY]]]
                &hex!("85410c411849001bc16d674ec800008242000342000482420005420006"),
                // [[DA],[GA],[ABvBbWdOyAAA],[[AAM],[AAQ]],[[AAU],[AAY]],[]]
                &hex!("86410c411849001bc16d674ec80000824200034200048242000542000640"),
            ),
        ];

        for (idx, (params, read_bytes, write_bytes)) in test_cases.into_iter().enumerate() {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, write_bytes, "Test case {} encoding failed", idx);

            let decoded: ExpirationSet =
                IpldBlock::deserialize(&IpldBlock { codec: 0x71, data: read_bytes.to_vec() })
                    .unwrap();
            assert_eq!(params, decoded, "Test case {} decoding failed", idx);
        }
    }

    #[test]
    fn deadline() {
        let test_cases = vec![
            (
                Deadline { ..Default::default() },
                // [baeaaaaa,baeaaaaa,[],[],0,0,[[],[]],baeaaaaa,baeaaaaa,baeaaaaa,baeaaaaa,[[],[]],[]]
                &hex!(
                    "8dd82a450001000000d82a45000100000040400000824040d82a450001000000d82a450001000000d82a450001000000d82a45000100000082404040"
                )[..],
            ),
            (
                Deadline {
                    partitions: Cid::from_str("bagboea4seaaqa").unwrap(),
                    expirations_epochs: Cid::from_str("bagboea4seaaqc").unwrap(),
                    partitions_posted: BitField::from_ranges(Ranges::new(
                        iter::once(0..1).collect::<Vec<Range<u64>>>(),
                    )),
                    early_terminations: BitField::from_ranges(Ranges::new(
                        iter::once(1..2).collect::<Vec<Range<u64>>>(),
                    )),
                    live_sectors: 2,
                    total_sectors: 3,
                    faulty_power: PowerPair::new(BigInt::from(4), BigInt::from(5)),
                    optimistic_post_submissions: Cid::from_str("bagboea4seaaqe").unwrap(),
                    sectors_snapshot: Cid::from_str("bagboea4seaaqg").unwrap(),
                    partitions_snapshot: Cid::from_str("bagboea4seaaqi").unwrap(),
                    optimistic_post_submissions_snapshot: Cid::from_str("bagboea4seaaqk").unwrap(),
                    live_power: PowerPair::new(BigInt::from(6), BigInt::from(7)),
                    daily_fee: TokenAmount::from_whole(8),
                },
                // [bagboea4seaaqa,bagboea4seaaqc,[DA],[GA],2,3,[[AAQ],[AAU]],bagboea4seaaqe,bagboea4seaaqg,bagboea4seaaqi,bagboea4seaaqk,[[AAY],[AAc]],[AG8FtZ07IAAA]]
                &hex!(
                    "8dd82a49000182e20392200100d82a49000182e20392200101410c4118020382420004420005d82a49000182e20392200102d82a49000182e20392200103d82a49000182e20392200104d82a49000182e203922001058242000642000749006f05b59d3b200000"
                ),
            ),
        ];

        for (params, expected) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(encoded.data, expected);
            let decoded: Deadline = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }
}
