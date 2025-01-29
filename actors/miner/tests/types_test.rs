// Tests to match with Go github.com/filecoin-project/go-state-types/builtin/*/miner
mod serialization {
    use std::str::FromStr;

    use cid::Cid;
    use fil_actor_miner::{ProveCommitSectorsNIParams, SectorNIActivationInfo, SectorOnChainInfo};
    use fvm_ipld_encoding::ipld_block::IpldBlock;
    use fvm_shared::econ::TokenAmount;
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

    /*
        pub struct SectorOnChainInfo {
        pub sector_number: SectorNumber,
        /// The seal proof type implies the PoSt proofs
        pub seal_proof: RegisteredSealProof,
        /// CommR
        pub sealed_cid: Cid,
        pub deprecated_deal_ids: Vec<DealID>,
        /// Epoch during which the sector proof was accepted
        pub activation: ChainEpoch,
        /// Epoch during which the sector expires
        pub expiration: ChainEpoch,
        /// Integral of active deals over sector lifetime
        #[serde(with = "bigint_ser")]
        pub deal_weight: DealWeight,
        /// Integral of active verified deals over sector lifetime
        #[serde(with = "bigint_ser")]
        pub verified_deal_weight: DealWeight,
        /// Pledge collected to commit this sector
        pub initial_pledge: TokenAmount,
        /// Expected one day projection of reward for sector computed at activation / update / extension time
        pub expected_day_reward: TokenAmount,
        /// Expected twenty day projection of reward for sector computed at activation / update / extension time
        pub expected_storage_pledge: TokenAmount,
        /// Epoch at which this sector's power was most recently updated
        pub power_base_epoch: ChainEpoch,
        /// Maximum day reward this sector has had in previous iterations (zero for brand new sectors)
        pub replaced_day_reward: TokenAmount,
        /// The original SealedSectorCID, only gets set on the first ReplicaUpdate
        pub sector_key_cid: Option<Cid>,
        /// Additional flags, see [`SectorOnChainInfoFlags`]
        pub flags: SectorOnChainInfoFlags,
        //// The fee to be burned during each PoSt submission, not present for sectors before nv25
        pub proving_period_fee: Option<TokenAmount>,
    }
     */

    #[test]
    fn sector_on_chain_info() {
        let test_cases = vec![
            (
                // TODO: same but with some None's in the struct
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
                    proving_period_fee: Some(TokenAmount::from_whole(11)),
                },
                "new",
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
                    sector_key_cid: None,
                    flags: Default::default(),
                    proving_period_fee: None,
                },
                "old",
                "8f0108d82a49000182e20392200100800203420004420005490053444835ec58000049006124fee993bc000049006f05b59d3b2000000949008ac7230489e80000f600",
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
