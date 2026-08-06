// Tests to match with Go github.com/filecoin-project/go-state-types/builtin/*/reward.
mod serialization {
    use cid::Cid;
    use fil_actor_reward::{
        ExplicitDistribution, PendingWrite, PendingWriteOp, RecipientAmount, RecipientShare, State,
        Stream, StreamAccrual, StreamsState, Tombstone, WeightRecord,
    };
    use fil_actors_runtime::reward::FilterEstimate;
    use fil_actors_runtime::test_blockstores::MemoryBlockstore;
    use fvm_ipld_encoding::{CborStore, RawBytes, ipld_block::IpldBlock};
    use fvm_shared::address::Address;
    use fvm_shared::bigint::BigInt;
    use fvm_shared::econ::TokenAmount;
    use hex_literal::hex;
    use multihash_codetable::Code;

    fn empty_streams_root() -> Cid {
        MemoryBlockstore::default().put_cbor(&StreamsState::default(), Code::Blake2b256).unwrap()
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
                        }],
                        claimed_period: vec![RecipientAmount {
                            recipient: Address::new_id(103),
                            amount: TokenAmount::from_atto(8),
                        }],
                    }),
                },
            ],
            tombstones: vec![Tombstone {
                id: 3,
                payable: vec![RecipientAmount {
                    recipient: Address::new_id(104),
                    amount: TokenAmount::from_atto(9),
                }],
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
    fn empty_reward_state_root() {
        let state = State {
            next_transition_epoch: -1,
            streams_root: empty_streams_root(),
            ..Default::default()
        };
        let encoded = IpldBlock::serialize_cbor(&state).unwrap().unwrap();
        assert_eq!(
            encoded.data,
            hex!(
                "8f40400040408240404000404040802000d82a5827000171a0e40220d63b11132be58f8f498e5f8c46c4d26b89675b443ff1c47f1e7e3d3cb8d2dcaa"
            )
        );
        let decoded: State = IpldBlock::deserialize(&encoded).unwrap();
        assert_eq!(-1, decoded.next_transition_epoch);
    }

    #[test]
    fn reward_state_root() {
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
            total_service_minted: TokenAmount::from_atto(10),
            service_accrued: vec![StreamAccrual { id: 2, amount: TokenAmount::from_atto(11) }],
            next_transition_epoch: 12,
            swa_timelock_epochs: 13,
            streams_root: empty_streams_root(),
        };

        let encoded = IpldBlock::serialize_cbor(&state).unwrap().unwrap();
        assert_eq!(
            encoded.data,
            hex!(
                "8f420001420002034200044200058240404200060742000842000942000a81820242000b0c0dd82a5827000171a0e40220d63b11132be58f8f498e5f8c46c4d26b89675b443ff1c47f1e7e3d3cb8d2dcaa"
            )
        );
        let decoded: State = IpldBlock::deserialize(&encoded).unwrap();
        assert_eq!(state.total_minted_reward, decoded.total_minted_reward);
        assert_eq!(state.streams_root, decoded.streams_root);
    }
}
