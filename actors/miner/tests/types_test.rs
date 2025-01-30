// Tests to match with Go github.com/filecoin-project/go-state-types/builtin/*/miner
mod serialization {
    use std::str::FromStr;

    use cid::Cid;
    use fil_actor_miner::{
        ProveCommitSectorsNIParams, SectorNIActivationInfo, SectorOnChainInfo,
        SectorOnChainInfoFlags,
    };
    use fvm_ipld_encoding::ipld_block::IpldBlock;
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
                },
                // [[],byte[],8,1,2,false]
                 "868040080102f4",
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
                },
                // [[[1,2,bagboea4seaaqa,3,4,5]],byte[deadbeef],18,1,6,true]
                "8681860102d82a49000182e2039220010003040544deadbeef120106f5",
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
                },
                // [[[1,2,bagboea4seaaqa,3,4,5],[6,7,bagboea4seaaqc,8,9,10]],byte[deadbeef],18,1,11,false]
                "8682860102d82a49000182e20392200100030405860607d82a49000182e2039220010108090a44deadbeef12010bf4",
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(const_hex::encode(&encoded.data), expected_hex);
            let decoded: ProveCommitSectorsNIParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }

    #[test]
    fn sector_on_chain_info() {
        let test_cases = vec![
            (
                SectorOnChainInfo {
                    ..Default::default()
                },
                "new",
                // [0,-1,{"/":"baeaaaaa"},[],0,0,[],[],[],[],[],0,[],null,0,[]]
                "900020d82a45000100000080000040404040400040f60040",
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
                    expected_day_reward: TokenAmount::from_whole(7),
                    expected_storage_pledge: TokenAmount::from_whole(8),
                    power_base_epoch: 9,
                    replaced_day_reward: TokenAmount::from_whole(10),
                    sector_key_cid: None,
                    flags: Default::default(),
                    proving_period_fee: TokenAmount::from_whole(11),
                },
                "new",
                // '[1,8,{"/":"bagboea4seaaqa"},[],2,3,[AAQ],[AAU],[AFNESDXsWAAA],[AGEk]TvAAA"}},[AG8FtZ07IAAA],9,[AIrHIwSJ6AAA],null,0,[AJin2bgxTAAA]]'
                "900108d82a49000182e20392200100800203420004420005490053444835ec58000049006124fee993bc000049006f05b59d3b2000000949008ac7230489e80000f600490098a7d9b8314c0000",
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
                    expected_day_reward: TokenAmount::from_whole(7),
                    expected_storage_pledge: TokenAmount::from_whole(8),
                    power_base_epoch: 9,
                    replaced_day_reward: TokenAmount::from_whole(10),
                    sector_key_cid: Some(Cid::from_str("baga6ea4seaaqc").unwrap()),
                    flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER,
                    proving_period_fee: TokenAmount::from_whole(11),
                },
                "new",
                // [1,8,{"/":"bagboea4seaaqa"},[],2,3,[AAQ],[AAU],[AFNESDXsWAAA],[AGEk]TvAAA"}},[AG8FtZ07IAAA],9,[AIrHIwSJ6AAA],{"/":"baga6ea4seaaqc"},1,[AJin2bgxTAAA]]
                "900108d82a49000182e20392200100800203420004420005490053444835ec58000049006124fee993bc000049006f05b59d3b2000000949008ac7230489e80000d82a49000181e2039220010101490098a7d9b8314c0000",
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
                    expected_day_reward: TokenAmount::from_whole(7),
                    expected_storage_pledge: TokenAmount::from_whole(8),
                    power_base_epoch: 9,
                    replaced_day_reward: TokenAmount::from_whole(10),
                    sector_key_cid: None,
                    flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER,
                    proving_period_fee: TokenAmount::zero(), // default, not present in the binary
                },
                "old",
                // [1,9,{"/":"bagboea4seaaqa"},[],2,3,[AAQ],[AAU],[AFNESDXsWAAA],[AGEk]TvAAA"}},[AG8FtZ07IAAA],9,[AIrHIwSJ6AAA],null,1]
                "8f0109d82a49000182e20392200100800203420004420005490053444835ec58000049006124fee993bc000049006f05b59d3b2000000949008ac7230489e80000f601",
            ),
        ];

        for (params, oldnew, expected_hex) in test_cases {
            if oldnew == "new" {
                let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
                assert_eq!(const_hex::encode(&encoded.data), expected_hex);
            }

            let byts = const_hex::decode(expected_hex).unwrap();
            let decoded: SectorOnChainInfo =
                IpldBlock::deserialize(&IpldBlock { codec: 0x71, data: byts }).unwrap();
            assert_eq!(params, decoded);
        }
    }
}
