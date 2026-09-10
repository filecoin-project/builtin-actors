use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;

use super::*;
use crate::streams::invariants::{accounting, structure};

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
        vec![RecipientAmount { recipient: delegated_address(), amount: TokenAmount::from_atto(1) }]
            .into();
    assert_invalid(streams);

    let mut streams = base.clone();
    streams.streams[1].distribution.as_mut().unwrap().claimed_period =
        vec![RecipientAmount { recipient: delegated_address(), amount: TokenAmount::from_atto(1) }]
            .into();
    assert_invalid(streams);

    let mut streams = base.clone();
    streams.tombstones = vec![Tombstone {
        id: 3,
        payable: vec![RecipientAmount {
            recipient: delegated_address(),
            amount: TokenAmount::from_atto(1),
        }]
        .into(),
    }];
    assert_invalid(streams);

    let mut streams = base.clone();
    streams.pending_writes_queue = vec![PendingWrite {
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
    streams.pending_writes_queue = vec![PendingWrite {
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
        pending_writes_queue: vec![PendingWrite {
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

#[test]
fn structural_validation_rejects_payable_reservation_over_cap() {
    let mut distribution = explicit(300, full_share_map(100));
    distribution.payable = (0..=MAX_PAYABLE_ROWS_PER_STREAM)
        .map(|idx| RecipientAmount {
            recipient: Address::new_id(100 + idx as u64),
            amount: TokenAmount::from_atto(1),
        })
        .collect::<Vec<_>>()
        .into();
    let streams = StreamsState {
        streams: vec![stream(2, pct(20), Some(distribution))],
        ..Default::default()
    };

    let error = structure(&streams).unwrap_err();
    assert_eq!(
        format!(
            "stream 2 payable row reservation {} exceeds maximum {MAX_PAYABLE_ROWS_PER_STREAM}",
            MAX_PAYABLE_ROWS_PER_STREAM + 1
        ),
        error.to_string()
    );
}

// Every explicit stream carries an accrual row, so a method aborts on a missing one rather than
// dropping the queued call that would have needed it.
#[test]
fn accounting_rejects_a_missing_accrual_row() {
    let (streams, _) = base_state();
    let error = accounting(&streams, &[]).unwrap_err();
    assert_eq!("explicit-stream accrual IDs do not match live explicit streams", error.to_string());
}

// The planted fault is the probe: `structure` rejects it and `accounting` accepts it, so an `Ok`
// from `validate_changes` proves `structure` did not run and the tombstone error proves it did.
// The test below is the other half, an accrual fault caught when only the accrual rows moved.
#[test]
fn validation_only_covers_changes() {
    let (mut streams, accruals) = base_state();
    // Plant a bad entry: empty tombstone is a `structure` fault, but `accounting`-only checks won't
    // catch this.
    streams.tombstones.push(Tombstone { id: 3, payable: RecipientTable::default() });
    assert!(structure(&streams).is_err()); // caught it
    accounting(&streams, &accruals).unwrap(); // passes

    let st = State { accrued: accruals.clone(), ..Default::default() };
    let mut ledger = ledger(&streams, &accruals);
    ledger.validate_changes(&st).unwrap(); // nothing changed, no invariants run

    // Trigger an `accounting` (only) check by modifying an accrual row.
    ledger.accrued[0].amount = TokenAmount::from_atto(1);
    ledger.validate_changes(&st).unwrap(); // only accounting invariants run, pass

    // The streams block moved so `structure` runs too.
    ledger.streams_dirty = true;
    let error = ledger.validate_changes(&st).unwrap_err();
    assert_eq!("tombstone 3 is empty", error.to_string());
}

#[test]
fn accrual_rows_are_checked_against_their_own_streams() {
    let (streams, accruals) = base_state();
    let st = State { accrued: accruals.clone(), ..Default::default() };
    let mut ledger = ledger(&streams, &accruals);
    ledger.accrued.push(StreamAccrual { id: 9, amount: TokenAmount::zero() });

    let error = ledger.validate_changes(&st).unwrap_err();
    assert_eq!("explicit-stream accrual IDs do not match live explicit streams", error.to_string());
}
