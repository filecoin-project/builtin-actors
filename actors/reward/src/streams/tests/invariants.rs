use fil_actors_runtime::{BURNT_FUNDS_ACTOR_ADDR, REWARD_ACTOR_ADDR};
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;

use super::*;
use crate::streams::invariants::structure;

fn delegated_address() -> Address {
    Address::new_delegated(10, &[1; 20]).unwrap()
}

#[test]
fn rejects_non_id_addresses_in_persisted_state_and_pending_payloads() {
    let base = base_state();
    let assert_invalid = |streams: StreamsState| {
        let error = validate_streams_state(&streams, 0).unwrap_err();
        assert!(error.to_string().contains("not an ID address"), "{error}");
    };

    let mut streams = base.clone();
    streams.streams[1].distribution.as_mut().unwrap().writer = delegated_address();
    assert_invalid(streams);

    let mut streams = base.clone();
    streams.streams[1].distribution.as_mut().unwrap().shares[0].recipient = delegated_address();
    assert_invalid(streams);

    let mut streams = base.clone();
    streams.pending_writes = vec![PendingWrite {
        id: Some(3),
        op: PendingWriteOp::RegisterStream,
        payload: RawBytes::serialize(&RegisterStreamPayload {
            weight: constant_weight(pct(10)),
            distribution: Some(DistributionInit {
                writer: delegated_address(),
                shares: shares(&[(103, DENOM)]),
            }),
        })
        .unwrap(),
        effective_epoch: 10,
    }];
    assert_invalid(streams);

    let mut streams = base;
    streams.pending_writes = vec![PendingWrite {
        id: Some(2),
        op: PendingWriteOp::SetDistribution,
        payload: RawBytes::serialize(&SetDistributionPayload { writer: delegated_address() })
            .unwrap(),
        effective_epoch: 10,
    }];
    assert_invalid(streams);
}

// Wire maps arrive in any order and admit_shares sorts them; a persisted map that is not
// ascending by recipient can only have been written by something other than f02.
#[test]
fn structural_validation_rejects_unordered_stored_shares() {
    let ordered = explicit(300, shares(&[(101, pct(50)), (102, pct(50))]));
    let streams = StreamsState {
        streams: vec![stream(2, pct(20), Some(ordered.clone()))],
        ..Default::default()
    };
    structure(&streams).unwrap();

    let mut unordered = ordered;
    unordered.shares.swap(0, 1);
    let streams =
        StreamsState { streams: vec![stream(2, pct(20), Some(unordered))], ..Default::default() };
    let error = structure(&streams).unwrap_err();
    assert_eq!("stored share recipients are not ordered", error.to_string());

    // The same rule gates the initial map carried by a pending registration.
    let streams = StreamsState {
        pending_writes: vec![PendingWrite {
            id: Some(3),
            op: PendingWriteOp::RegisterStream,
            payload: RawBytes::serialize(&RegisterStreamPayload {
                weight: constant_weight(pct(10)),
                distribution: Some(DistributionInit {
                    writer: Address::new_id(300),
                    shares: shares(&[(102, pct(50)), (101, pct(50))]),
                }),
            })
            .unwrap(),
            effective_epoch: 1,
        }],
        ..Default::default()
    };
    let error = structure(&streams).unwrap_err();
    assert!(error.to_string().contains("stored share recipients are not ordered"), "{error}");
}

// Admission never stores the burn sentinel or the reward actor, so a persisted map holding either
// is corrupt.
#[test]
fn structural_validation_rejects_persisted_sentinel_and_reward_actor_recipients() {
    let base = base_state();
    validate_streams_state(&base, 0).unwrap();
    let with_recipient = |recipient: Address| {
        let mut streams = base.clone();
        streams.streams[1].distribution.as_mut().unwrap().shares =
            vec![RecipientShare { recipient, share: DENOM }];
        streams
    };

    assert!(validate_streams_state(&with_recipient(REWARD_ACTOR_ADDR), 0).is_err());
    assert!(validate_streams_state(&with_recipient(BURNT_FUNDS_ACTOR_ADDR), 0).is_err());
}
