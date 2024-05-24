// Tests to match with Go github.com/filecoin-project/go-state-types/builtin/*/miner
mod serialization {
    use std::str::FromStr;

    use cid::Cid;
    use fil_actor_miner::{ProveCommitSectorsNIParams, SectorNIActivationInfo};
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::sector::{RegisteredAggregateProof, RegisteredSealProof};

    #[test]
    fn prove_commit_sectors_ni_params() {
        let test_cases = vec![
            (
                ProveCommitSectorsNIParams {
                    sectors: vec![],
                    seal_proof_type: RegisteredSealProof::StackedDRG32GiBV1P1,
                    sector_proofs: vec![],
                    aggregate_proof: vec![].into(),
                    aggregate_proof_type: None,
                    require_activation_success: false,
                },
                // [[],8,[],byte[],null,false]
                "8680088040f6f4",
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
                    seal_proof_type: RegisteredSealProof::StackedDRG32GiBV1P1_Feat_NiPoRep,
                    sector_proofs: vec![],
                    aggregate_proof: vec![0xde, 0xad, 0xbe, 0xef].into(),
                    aggregate_proof_type: Some(RegisteredAggregateProof::SnarkPackV2),
                    require_activation_success: true,
                },
                // [[[1,2,bagboea4seaaqa,3,4,5]],18,[],byte[deadbeef],1,true]
                "8681860102d82a49000182e20392200100030405128044deadbeef01f5",
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
                    seal_proof_type: RegisteredSealProof::StackedDRG32GiBV1P1_Feat_NiPoRep,
                    sector_proofs: vec![vec![0xde, 0xad].into(), vec![0xbe, 0xef].into()],
                    aggregate_proof: vec![].into(),
                    aggregate_proof_type: None,
                    require_activation_success: false,
                },
                // [[[1,2,bagboea4seaaqa,3,4,5],[6,7,bagboea4seaaqc,8,9,10]],18,[byte[dead],byte[beef]],byte[],null,false]
                "8682860102d82a49000182e20392200100030405860607d82a49000182e2039220010108090a128242dead42beef40f6f4",
            ),
        ];

        for (params, expected_hex) in test_cases {
            let encoded = IpldBlock::serialize_cbor(&params).unwrap().unwrap();
            assert_eq!(const_hex::encode(&encoded.data), expected_hex);
            let decoded: ProveCommitSectorsNIParams = IpldBlock::deserialize(&encoded).unwrap();
            assert_eq!(params, decoded);
        }
    }
}
