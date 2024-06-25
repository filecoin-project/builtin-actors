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
}
