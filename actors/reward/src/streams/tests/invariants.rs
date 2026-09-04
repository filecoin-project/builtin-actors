use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;

use super::*;

fn delegated_address() -> Address {
    Address::new_delegated(10, &[1; 20]).unwrap()
}

#[test]
fn rejects_non_id_addresses_in_persisted_state_and_pending_payloads() {
    let (base, accruals) = base_state();
    let assert_invalid = |streams: StreamsState| {
        let error = validate_streams_state(&streams, &accruals, 0).unwrap_err();
        assert!(error.to_string().contains("not an ID address"), "{error}");
    };

    let mut streams = base.clone();
    streams.streams[1].distribution.as_mut().unwrap().writer = delegated_address();
    assert_invalid(streams);

    let mut streams = base.clone();
    streams.streams[1].distribution.as_mut().unwrap().shares[0].recipient = delegated_address();
    assert_invalid(streams);

    let mut streams = base.clone();
    streams.streams[1].distribution.as_mut().unwrap().payable =
        vec![RecipientAmount { recipient: delegated_address(), amount: TokenAmount::from_atto(1) }];
    assert_invalid(streams);

    let mut streams = base.clone();
    streams.streams[1].distribution.as_mut().unwrap().claimed_period =
        vec![RecipientAmount { recipient: delegated_address(), amount: TokenAmount::from_atto(1) }];
    assert_invalid(streams);

    let mut streams = base.clone();
    streams.tombstones = vec![Tombstone {
        id: 3,
        payable: vec![RecipientAmount {
            recipient: delegated_address(),
            amount: TokenAmount::from_atto(1),
        }],
    }];
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

#[test]
fn rejects_persisted_tombstone_rows_above_the_bound() {
    let (mut streams, accruals) = base_state();
    streams.tombstones = vec![tombstone(3, 1_000, MAX_TOMBSTONE_ROWS + 1)];
    let error = validate_streams_state(&streams, &accruals, 0).unwrap_err();
    assert!(error.to_string().contains("tombstone row reservation 257"), "{error}");
}

// Wire maps arrive in any order and normalize_shares sorts them; a persisted map that is not
// ascending by recipient can only have been written by something other than f02.
#[test]
fn structural_validation_rejects_unordered_stored_shares() {
    let ordered = explicit(300, shares(&[(101, pct(50)), (102, pct(50))]));
    let streams = StreamsState {
        streams: vec![stream(2, pct(20), Some(ordered.clone()))],
        ..Default::default()
    };
    validate_award_state_structure(&streams).unwrap();

    let mut unordered = ordered;
    unordered.shares.swap(0, 1);
    let streams =
        StreamsState { streams: vec![stream(2, pct(20), Some(unordered))], ..Default::default() };
    let error = validate_award_state_structure(&streams).unwrap_err();
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
    let error = validate_award_state_structure(&streams).unwrap_err();
    assert!(error.to_string().contains("stored share recipients are not ordered"), "{error}");
}

#[test]
fn structural_validation_rejects_payable_reservation_over_cap() {
    let mut distribution = explicit(300, full_share_map(100));
    distribution.payable = (0..=MAX_PAYABLE_ROWS_PER_STREAM)
        .map(|idx| RecipientAmount {
            recipient: Address::new_id(100 + idx as u64),
            amount: TokenAmount::from_atto(1),
        })
        .collect();
    let streams = StreamsState {
        streams: vec![stream(2, pct(20), Some(distribution))],
        ..Default::default()
    };

    let error = validate_award_state_structure(&streams).unwrap_err();
    assert_eq!(
        format!(
            "stream 2 payable row reservation {} exceeds maximum {MAX_PAYABLE_ROWS_PER_STREAM}",
            MAX_PAYABLE_ROWS_PER_STREAM + 1
        ),
        error.to_string()
    );
}
