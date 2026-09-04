use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_shared::address::Address;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::econ::TokenAmount;
use hex_literal::hex;
use num_traits::Zero;

use super::*;
use crate::streams::invariants::structure;
use crate::streams::weights::{WeightRecordsPayload, compute_weight};

fn next_epoch(streams: &StreamsState) -> ChainEpoch {
    streams.pending_writes.first().map_or(EPOCH_UNDEFINED, |write| write.effective_epoch)
}

fn random_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *state
}

#[test]
fn deferred_payloads_have_stable_cbor() {
    let weight = WeightRecord { v_start: 2, slope: -1, t_start: 3, floor: 0, cap: 4 };
    let batch = WeightRecordsPayload {
        updates: vec![
            WeightRecordUpdate { id: 1, weight: weight.clone() },
            WeightRecordUpdate {
                id: 5,

                weight: WeightRecord { v_start: 8, slope: 0, t_start: 7, floor: 8, cap: 9 },
            },
        ],
    };

    assert_eq!(
        hex!("850220030004").as_slice(),
        fvm_ipld_encoding::RawBytes::serialize(&weight).unwrap().bytes()
    );
    assert_eq!(
        hex!("818282018502200300048205850800070809").as_slice(),
        fvm_ipld_encoding::RawBytes::serialize(&batch).unwrap().bytes()
    );
    assert_eq!(
        hex!("82850220030004f6").as_slice(),
        fvm_ipld_encoding::RawBytes::serialize(&RegisterStreamPayload {
            weight,
            distribution: None,
        })
        .unwrap()
        .bytes()
    );
    assert_eq!(
        hex!("82850220030004824300c80181824200651b0de0b6b3a7640000").as_slice(),
        fvm_ipld_encoding::RawBytes::serialize(&RegisterStreamPayload {
            weight: WeightRecord { v_start: 2, slope: -1, t_start: 3, floor: 0, cap: 4 },
            distribution: Some(DistributionInit {
                writer: Address::new_id(200),
                shares: shares(&[(101, DENOM)]),
            }),
        })
        .unwrap()
        .bytes()
    );
    assert_eq!(
        hex!("81420001").as_slice(),
        fvm_ipld_encoding::RawBytes::serialize(&SetDistributionPayload {
            writer: Address::new_id(1),
        })
        .unwrap()
        .bytes()
    );
    assert_eq!(hex!("80").as_slice(), fvm_ipld_encoding::RawBytes::new(vec![0x80]).bytes());
    assert_eq!(
        hex!("84f60041810a").as_slice(),
        fvm_ipld_encoding::RawBytes::serialize(&PendingWrite {
            id: None,
            op: PendingWriteOp::SetWeightRecords,
            payload: fvm_ipld_encoding::RawBytes::new(vec![0x81]),
            effective_epoch: 10,
        })
        .unwrap()
        .bytes()
    );
}

#[test]
fn removal_admission_reserves_tombstone_rows_and_claim_relieves_the_bound() {
    let (base, accruals) = base_state();

    let mut at_boundary = base.clone();
    at_boundary.tombstones = vec![tombstone(3, 1_000, 192)];
    queue_remove_stream(&mut at_boundary, &accruals, 0, 1, 2).unwrap();

    let mut over = base;
    over.tombstones = vec![tombstone(3, 1_000, 193)];
    let error = queue_remove_stream(&mut over, &accruals, 0, 1, 2).unwrap_err();
    assert!(error.to_string().contains("tombstone row reservation 257"), "{error}");

    let paid = claim(&mut over, &accruals, 3, &[Address::new_id(1_000)]).unwrap();
    assert_eq!(vec![TokenAmount::from_atto(1)], paid);
    queue_remove_stream(&mut over, &accruals, 0, 1, 2).unwrap();
}

#[test]
fn pending_removal_bound_survives_share_folds_during_the_hold() {
    let (mut streams, mut accruals) = base_state();
    streams.tombstones = vec![tombstone(3, 1_000, 192)];
    accruals[0].amount = TokenAmount::from_atto(1);
    queue_remove_stream(&mut streams, &accruals, 0, 1, 2).unwrap();

    let share = DENOM / MAX_RECIPIENTS as u64;
    let replacement: Vec<_> = (0..MAX_RECIPIENTS)
        .map(|offset| RecipientShare { recipient: Address::new_id(2_000 + offset as u64), share })
        .collect();
    // A rejected fold leaves the ledger unspecified, so the caller discards it; here that is a
    // copy, and the state the rest of the test uses is the one that was never folded.
    let error = set_shares(&mut streams.clone(), &mut accruals.clone(), 2, replacement.clone())
        .unwrap_err();
    assert!(error.to_string().contains("tombstone row reservation 257"), "{error}");

    claim(&mut streams, &accruals, 3, &[Address::new_id(1_000)]).unwrap();
    set_shares(&mut streams, &mut accruals, 2, replacement).unwrap();
    apply_due_writes(&mut streams, &mut accruals, 1);
    let rows: usize = streams.tombstones.iter().map(|row| row.payable.len()).sum();
    assert!(rows <= MAX_TOMBSTONE_ROWS);
}

#[test]
fn rejects_out_of_band_anchors_on_every_weight_queue_operation() {
    let invalid = [
        ("below floor", WeightRecord { v_start: 10, slope: 1, t_start: 0, floor: 20, cap: 30 }),
        ("exceeds cap", WeightRecord { v_start: 30, slope: -1, t_start: 0, floor: 10, cap: 20 }),
    ];

    for op in [PendingWriteOp::SetWeightRecords, PendingWriteOp::StepWeightRecords] {
        for (message, weight) in &invalid {
            let (mut streams, accruals) = base_state();
            let before = streams.clone();
            let error = queue_weight_records(
                &mut streams,
                &accruals,
                0,
                1,
                op,
                &[WeightRecordUpdate { id: 1, weight: weight.clone() }],
            )
            .unwrap_err();
            assert!(error.to_string().contains(message));
            assert_eq!(before, streams);
            assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
        }
    }

    for (message, weight) in invalid {
        let (mut streams, accruals) = base_state();
        let before = streams.clone();
        let error = queue_register_stream(
            &mut streams,
            &accruals,
            0,
            1,
            Stream { id: 3, weight, distribution: None },
            1,
        )
        .unwrap_err();
        assert!(error.to_string().contains(message));
        assert_eq!(before, streams);
        assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
    }
}

#[test]
fn burns_sentinel_share_without_accruing_or_tombstoning_it() {
    let full_shares = shares(&[(101, pct(25)), (102, pct(75))]);
    let mut streams = StreamsState {
        streams: vec![
            stream(1, pct(60), None),
            stream(2, pct(20), Some(explicit(200, full_shares))),
        ],
        ..Default::default()
    };
    let mut accruals = vec![StreamAccrual { id: 2, amount: TokenAmount::zero() }];
    let reward = TokenAmount::from_atto(100);

    let common = allocate(&streams.streams, 0, &reward);
    assert_eq!(TokenAmount::from_atto(60), common.miner);
    assert_eq!(TokenAmount::from_atto(20), common.portions[0].1);
    assert_eq!(TokenAmount::from_atto(20), common.burn);

    set_shares(
        &mut streams,
        &mut accruals,
        2,
        shares(&[(99, pct(25)), (101, pct(25)), (99, pct(25)), (102, pct(25))]),
    )
    .unwrap();
    let distribution = streams.streams[1].distribution.as_ref().unwrap();
    assert_eq!(shares(&[(101, pct(25)), (102, pct(25))]), distribution.shares);

    let partial = allocate(&streams.streams, 0, &reward);
    assert_eq!(TokenAmount::from_atto(60), partial.miner);
    assert_eq!(TokenAmount::from_atto(10), partial.portions[0].1);
    assert_eq!(TokenAmount::from_atto(30), partial.burn);
    assert_eq!(reward, &partial.miner + &partial.portions[0].1 + &partial.burn);
    accrue(&mut accruals, &partial.portions);

    // f099 is included to prove the sentinel has no claimable balance. Recipient 102 is omitted
    // so its entitlement is folded into payable below.
    let claimed =
        claim(&mut streams, &accruals, 2, &[Address::new_id(101), BURNT_FUNDS_ACTOR_ADDR]).unwrap();
    assert_eq!(vec![TokenAmount::from_atto(5), TokenAmount::zero()], claimed);
    let distribution = streams.streams[1].distribution.as_ref().unwrap();
    assert_eq!(vec![Address::new_id(101)], {
        distribution.claimed_period.iter().map(|row| row.recipient).collect::<Vec<_>>()
    });

    set_shares(&mut streams, &mut accruals, 2, shares(&[(99, DENOM)])).unwrap();
    let distribution = streams.streams[1].distribution.as_ref().unwrap();
    assert!(distribution.shares.is_empty());
    assert!(distribution.claimed_period.is_empty());
    assert_eq!(vec![Address::new_id(102)], {
        distribution.payable.iter().map(|row| row.recipient).collect::<Vec<_>>()
    });

    let removed = allocate(&streams.streams, 0, &reward);
    assert_eq!(TokenAmount::from_atto(60), removed.miner);
    assert_eq!(TokenAmount::zero(), removed.portions[0].1);
    assert_eq!(TokenAmount::from_atto(40), removed.burn);
    assert_eq!(reward, &removed.miner + &removed.portions[0].1 + &removed.burn);
    accrue(&mut accruals, &removed.portions);

    queue_remove_stream(&mut streams, &accruals, 0, 1, 2).unwrap();
    apply_due_writes(&mut streams, &mut accruals, 1);
    assert_eq!(vec![Address::new_id(102)], {
        streams.tombstones[0].payable.iter().map(|row| row.recipient).collect::<Vec<_>>()
    });
    let claimed =
        claim(&mut streams, &accruals, 2, &[BURNT_FUNDS_ACTOR_ADDR, Address::new_id(102)]).unwrap();
    assert_eq!(vec![TokenAmount::zero(), TokenAmount::from_atto(5)], claimed);
    assert!(streams.tombstones.is_empty());
}

#[test]
fn caps_payable_rows_at_128_rejects_129_atomically_and_recovers_through_claims() {
    let old_shares = full_share_map(100);
    let new_shares = full_share_map(200);
    let mut distribution = explicit(300, old_shares.clone());
    distribution.payable = old_shares
        .iter()
        .map(|row| RecipientAmount { recipient: row.recipient, amount: TokenAmount::from_atto(1) })
        .chain([RecipientAmount {
            recipient: Address::new_id(1_000),
            amount: TokenAmount::from_atto(1),
        }])
        .collect::<Vec<_>>()
        .into();
    let mut streams = StreamsState {
        streams: vec![stream(2, pct(20), Some(distribution))],
        ..Default::default()
    };
    let mut accruals =
        vec![StreamAccrual { id: 2, amount: TokenAmount::from_atto(MAX_RECIPIENTS as u64) }];

    let streams_before = streams.clone();
    let accruals_before = accruals.clone();
    let error = set_shares(&mut streams, &mut accruals, 2, new_shares.clone()).unwrap_err();
    assert_eq!(
        format!(
            "stream 2 payable row reservation {} exceeds maximum {MAX_PAYABLE_ROWS_PER_STREAM}",
            MAX_PAYABLE_ROWS_PER_STREAM + 1
        ),
        error.to_string()
    );
    assert_eq!(streams_before, streams);
    assert_eq!(accruals_before, accruals);

    assert_eq!(
        vec![TokenAmount::from_atto(1)],
        claim(&mut streams, &accruals, 2, &[Address::new_id(1_000)]).unwrap()
    );
    set_shares(&mut streams, &mut accruals, 2, new_shares.clone()).unwrap();
    let distribution = streams.streams[0].distribution.as_ref().unwrap();
    assert_eq!(MAX_PAYABLE_ROWS_PER_STREAM, distribution.payable.union_len(&distribution.shares));

    accruals[0].amount = TokenAmount::from_atto(MAX_RECIPIENTS as u64);
    set_shares(&mut streams, &mut accruals, 2, new_shares.clone()).unwrap();
    let distribution = streams.streams[0].distribution.as_ref().unwrap();
    assert_eq!(MAX_PAYABLE_ROWS_PER_STREAM, distribution.payable.len());
    let mut writer_streams = streams.clone();
    let mut writer_accruals = accruals.clone();
    writer_accruals[0].amount = TokenAmount::from_atto(MAX_RECIPIENTS as u64);
    replace_writer(&mut writer_streams, &mut writer_accruals, 2, Address::new_id(301)).unwrap();
    assert_eq!(
        MAX_PAYABLE_ROWS_PER_STREAM,
        writer_streams.streams[0].distribution.as_ref().unwrap().payable.len()
    );

    let mut removed_streams = streams.clone();
    let mut removed_accruals = accruals.clone();
    removed_accruals[0].amount = TokenAmount::from_atto(MAX_RECIPIENTS as u64);
    remove_stream(&mut removed_streams, &mut removed_accruals, 2).unwrap();
    assert_eq!(MAX_PAYABLE_ROWS_PER_STREAM, removed_streams.tombstones[0].payable.len());

    let old_wallets: Vec<_> = old_shares.iter().map(|row| row.recipient).collect();
    let new_wallets: Vec<_> = new_shares.iter().map(|row| row.recipient).collect();
    let first_batch = claim(&mut streams, &accruals, 2, &old_wallets).unwrap();
    assert_eq!(MAX_RECIPIENTS, first_batch.len());
    assert!(first_batch.iter().all(|amount| amount == &TokenAmount::from_atto(2)));
    assert_eq!(MAX_RECIPIENTS, streams.streams[0].distribution.as_ref().unwrap().payable.len());
    let second_batch = claim(&mut streams, &accruals, 2, &new_wallets).unwrap();
    assert_eq!(MAX_RECIPIENTS, second_batch.len());
    assert!(second_batch.iter().all(|amount| amount == &TokenAmount::from_atto(1)));
    assert!(streams.streams[0].distribution.as_ref().unwrap().payable.is_empty());
}

#[test]
fn rejects_a_new_call_that_strands_an_existing_call() {
    let (mut streams, accruals) = base_state();

    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        30,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(40)) }],
    )
    .unwrap();
    assert_eq!(30, next_epoch(&streams));

    let error = queue_weight_records(
        &mut streams,
        &accruals,
        10,
        10,
        PendingWriteOp::StepWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(70)) }],
    )
    .unwrap_err();
    assert!(error.to_string().contains("invalidates an existing pending call"));
    assert_eq!(1, streams.pending_writes.len());
}

#[test]
fn removal_rejects_stranding_an_inflight_gate_write() {
    let (mut streams, mut accruals) = base_state();
    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        2,
        PendingWriteOp::StepWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(30)) }],
    )
    .unwrap();

    let before = streams.clone();
    let error = queue_remove_stream(&mut streams, &accruals, 0, 1, 2).unwrap_err();
    assert!(error.to_string().contains("invalidates an existing pending call"));
    assert_eq!(before, streams);

    let result = apply_due_writes(&mut streams, &mut accruals, 2);
    assert_eq!(
        vec![PendingWriteOp::StepWeightRecords],
        result.applied.iter().map(|w| w.op).collect::<Vec<_>>()
    );
    assert!(result.dropped.is_empty());
    assert_eq!(pct(30), streams.streams[1].weight.v_start);

    queue_remove_stream(&mut streams, &accruals, 2, 1, 2).unwrap();
    assert_eq!(3, next_epoch(&streams));
}

#[test]
fn rejects_a_schedule_that_depends_on_a_later_call() {
    let (mut streams, accruals) = base_state();

    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        20,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(20)) }],
    )
    .unwrap();
    let error = queue_weight_records(
        &mut streams,
        &accruals,
        0,
        10,
        PendingWriteOp::StepWeightRecords,
        &[WeightRecordUpdate {
            id: 2,
            weight: WeightRecord {
                v_start: pct(20),
                slope: pct(1) as i64,
                t_start: 10,
                floor: pct(20),
                cap: pct(50),
            },
        }],
    )
    .unwrap_err();

    assert!(error.to_string().contains("exceed DENOM"));
    assert_eq!(1, streams.pending_writes.len());
    assert_eq!(20, next_epoch(&streams));
}

#[test]
fn rejects_single_epoch_overlap_on_either_side_of_a_clamp_crossing() {
    struct Case {
        name: &'static str,
        updates: [WeightRecordUpdate; 2],
        crossing_epoch: ChainEpoch,
        violating_epoch: ChainEpoch,
    }

    let cases = [
        Case {
            name: "before a falling stream leaves its cap",
            updates: [
                WeightRecordUpdate {
                    id: 1,
                    weight: WeightRecord {
                        v_start: pct(12),
                        slope: -(pct(40) as i64),
                        t_start: 14,
                        floor: 0,
                        cap: pct(90),
                    },
                },
                WeightRecordUpdate {
                    id: 2,
                    weight: WeightRecord {
                        v_start: pct(21),
                        slope: pct(5) as i64,
                        t_start: 14,
                        floor: 0,
                        cap: DENOM,
                    },
                },
            ],
            crossing_epoch: 13,
            violating_epoch: 12,
        },
        Case {
            name: "after a rising stream reaches its cap",
            updates: [
                WeightRecordUpdate {
                    id: 1,
                    weight: WeightRecord {
                        v_start: pct(31),
                        slope: pct(40) as i64,
                        t_start: 11,
                        floor: 0,
                        cap: pct(81),
                    },
                },
                WeightRecordUpdate {
                    id: 2,
                    weight: WeightRecord {
                        v_start: pct(35),
                        slope: -(pct(5) as i64),
                        t_start: 10,
                        floor: 0,
                        cap: DENOM,
                    },
                },
            ],
            crossing_epoch: 12,
            violating_epoch: 13,
        },
    ];

    for case in cases {
        assert_eq!(1, (case.violating_epoch - case.crossing_epoch).abs(), "{}", case.name);
        let weight_sum = |epoch| {
            case.updates
                .iter()
                .map(|update| u128::from(compute_weight(&update.weight, epoch)))
                .sum::<u128>()
        };
        assert!(weight_sum(case.violating_epoch - 1) <= u128::from(DENOM), "{}", case.name);
        assert!(weight_sum(case.violating_epoch) > u128::from(DENOM), "{}", case.name);
        assert!(weight_sum(case.violating_epoch + 1) <= u128::from(DENOM), "{}", case.name);

        let (mut streams, accruals) = base_state();
        let error = queue_weight_records(
            &mut streams,
            &accruals,
            0,
            10,
            PendingWriteOp::SetWeightRecords,
            &case.updates,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains(&format!("exceed DENOM at epoch {}", case.violating_epoch)),
            "{}: {error}",
            case.name
        );
        assert!(streams.pending_writes.is_empty(), "{}", case.name);
    }
}

#[test]
fn queues_batches_cancels_slots_and_tracks_queue_head() {
    let (mut streams, mut accruals) = base_state();
    let updates = [
        WeightRecordUpdate { id: 1, weight: constant_weight(pct(65)) },
        WeightRecordUpdate { id: 2, weight: constant_weight(pct(35)) },
    ];

    assert_eq!(
        17,
        queue_weight_records(
            &mut streams,
            &accruals,
            10,
            7,
            PendingWriteOp::SetWeightRecords,
            &updates,
        )
        .unwrap()
        .effective_epoch
    );
    assert_eq!(17, next_epoch(&streams));
    assert_eq!(1, streams.pending_writes.len());
    assert_eq!(None, streams.pending_writes[0].id);
    let payload: WeightRecordsPayload = streams.pending_writes[0].payload.deserialize().unwrap();
    assert_eq!(updates, payload.updates.as_slice());
    assert!(
        queue_weight_records(
            &mut streams,
            &accruals,
            10,
            7,
            PendingWriteOp::SetWeightRecords,
            &updates[..1],
        )
        .is_err()
    );

    assert!(cancel(&mut streams, Some(999), PendingWriteOp::SetWeightRecords).is_err());
    assert_eq!(17, next_epoch(&streams));
    assert!(cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some());
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
    assert!(cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_none());
    assert!(cancel(&mut streams, None, PendingWriteOp::RemoveStream).is_err());

    queue_weight_records(
        &mut streams,
        &accruals,
        20,
        7,
        PendingWriteOp::SetWeightRecords,
        &updates[1..],
    )
    .unwrap();
    assert_eq!(27, next_epoch(&streams));
    assert!(cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some());
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));

    queue_weight_records(
        &mut streams,
        &accruals,
        10,
        7,
        PendingWriteOp::StepWeightRecords,
        &updates[..1],
    )
    .unwrap();
    assert!(cancel(&mut streams, None, PendingWriteOp::StepWeightRecords).is_err());
    let result = apply_due_writes(&mut streams, &mut accruals, 17);
    let removed = cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap();
    assert_eq!(TokenAmount::zero(), result.fold_dust);
    assert_eq!(
        vec![PendingWriteOp::StepWeightRecords],
        result.applied.iter().map(|write| write.op).collect::<Vec<_>>()
    );
    assert!(result.dropped.is_empty());
    assert!(removed.is_none());
    assert_eq!(pct(65), streams.streams[0].weight.v_start);

    assert_eq!(
        17,
        queue_weight_records(
            &mut streams,
            &accruals,
            17,
            0,
            PendingWriteOp::SetWeightRecords,
            &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(30)) }],
        )
        .unwrap()
        .effective_epoch
    );
    assert_eq!(17, next_epoch(&streams));
    apply_due_writes(&mut streams, &mut accruals, 17);
    assert_eq!(pct(30), streams.streams[1].weight.v_start);
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
}

#[test]
fn apply_and_cancel_surfaces_calls_dropped_before_cancellation() {
    let (mut streams, mut accruals) = base_state();
    queue_register_stream(
        &mut streams,
        &accruals,
        0,
        10,
        stream(3, 0, Some(explicit(203, shares(&[(103, DENOM)])))),
        10,
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 3, weight: constant_weight(pct(10)) }],
    )
    .unwrap();
    assert!(cancel(&mut streams, Some(3), PendingWriteOp::RegisterStream).unwrap().is_some());

    let result = apply_due_writes(&mut streams, &mut accruals, 10);
    let removed = cancel(&mut streams, Some(99), PendingWriteOp::RemoveStream).unwrap();
    assert_eq!(1, result.dropped.len());
    assert_eq!(PendingWriteOp::SetWeightRecords, result.dropped[0].op);
    assert!(removed.is_none());
    assert!(streams.pending_writes.is_empty());
}

#[test]
fn enforces_registration_bounds_and_id_availability() {
    let (mut streams, accruals) = base_state();
    let new_stream =
        stream(3, 0, Some(explicit(201, shares(&[(103, DENOM / 2), (102, DENOM - DENOM / 2)]))));

    assert!(queue_register_stream(&mut streams, &accruals, 10, 7, new_stream.clone(), 16).is_err());
    assert_eq!(
        17,
        queue_register_stream(&mut streams, &accruals, 10, 7, new_stream, 17)
            .unwrap()
            .effective_epoch
    );
    let payload: RegisterStreamPayload = streams.pending_writes[0].payload.deserialize().unwrap();
    assert_eq!(
        vec![Address::new_id(102), Address::new_id(103)],
        payload.distribution.unwrap().shares.iter().map(|row| row.recipient).collect::<Vec<_>>()
    );
    assert!(
        queue_register_stream(
            &mut streams,
            &accruals,
            10,
            7,
            stream(3, 0, Some(explicit(201, shares(&[(102, DENOM)])))),
            17,
        )
        .is_err()
    );

    let mut full = StreamsState::default();
    for id in 1..=MAX_STREAMS as u64 {
        full.streams.push(stream(
            id,
            0,
            (id != 1).then(|| explicit(200 + id, shares(&[(100 + id, DENOM)]))),
        ));
    }
    assert_eq!(MAX_STREAMS, full.streams.len());
    let full_accruals: Vec<_> = (2..=MAX_STREAMS as u64)
        .map(|id| StreamAccrual { id, amount: TokenAmount::zero() })
        .collect();
    assert!(
        queue_register_stream(
            &mut full,
            &full_accruals,
            0,
            1,
            stream(9, 0, Some(explicit(209, shares(&[(109, DENOM)])))),
            1,
        )
        .is_err()
    );

    let mut seven = StreamsState::default();
    for id in 1..MAX_STREAMS as u64 {
        seven.streams.push(stream(
            id,
            0,
            (id != 1).then(|| explicit(200 + id, shares(&[(100 + id, DENOM)]))),
        ));
    }
    let mut seven_accruals: Vec<_> = (2..MAX_STREAMS as u64)
        .map(|id| StreamAccrual { id, amount: TokenAmount::zero() })
        .collect();
    queue_register_stream(
        &mut seven,
        &seven_accruals,
        0,
        1,
        stream(8, 0, Some(explicit(208, shares(&[(108, DENOM)])))),
        1,
    )
    .unwrap();
    apply_due_writes(&mut seven, &mut seven_accruals, 1);
    assert_eq!(MAX_STREAMS, seven.streams.len());

    let mut tombstoned = StreamsState {
        tombstones: vec![Tombstone {
            id: 4,
            payable: vec![RecipientAmount {
                recipient: Address::new_id(101),
                amount: TokenAmount::from_atto(1),
            }]
            .into(),
        }],
        ..Default::default()
    };
    assert!(
        queue_register_stream(
            &mut tombstoned,
            &[],
            0,
            1,
            stream(4, 0, Some(explicit(204, shares(&[(104, DENOM)])))),
            1,
        )
        .is_err()
    );

    let claimed = claim(&mut tombstoned, &[], 4, &[Address::new_id(101)]).unwrap();
    assert_eq!(vec![TokenAmount::from_atto(1)], claimed);
    assert!(tombstoned.tombstones.is_empty());
    assert_eq!(
        1,
        queue_register_stream(
            &mut tombstoned,
            &[],
            0,
            1,
            stream(4, 0, Some(explicit(204, shares(&[(104, DENOM)])))),
            1,
        )
        .unwrap()
        .effective_epoch
    );
}

#[test]
fn rejects_invalid_timelocks_without_mutation() {
    let (mut streams, accruals) = base_state();
    let before = streams.clone();

    assert!(queue_remove_stream(&mut streams, &accruals, 0, -1, 2).is_err());
    assert!(queue_remove_stream(&mut streams, &accruals, i64::MAX, 1, 2).is_err());
    assert_eq!(before, streams);
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
}

#[test]
fn bounds_the_pending_queue() {
    let writes = (0..=MAX_PENDING_WRITES)
        .map(|idx| PendingWrite {
            id: None,
            op: PendingWriteOp::SetWeightRecords,
            payload: fvm_ipld_encoding::RawBytes::new(Vec::new()),
            effective_epoch: idx as i64 + 1,
        })
        .collect::<Vec<_>>();
    let streams = StreamsState { pending_writes: writes, ..Default::default() };

    let error = structure(&streams).unwrap_err();
    assert!(error.to_string().contains("pending write count"), "{error}");
}

// Admission is where the queue grows, and the slot, ordering and payload guards around it leave
// only its length for this one to hold.
#[test]
fn admission_bounds_the_pending_queue() {
    let mut streams = StreamsState::default();
    // Every stream here is registered by a pending write, so the projection opens its own
    // accrual row and none is persisted yet.
    let accruals: Vec<StreamAccrual> = Vec::new();
    let register = |streams: &mut StreamsState, id: StreamId| {
        queue_register_stream(
            streams,
            &accruals,
            0,
            1,
            stream(id, 0, Some(explicit(200 + id, shares(&[(100 + id, DENOM)])))),
            1,
        )
        .unwrap();
    };
    for id in 3..=10 {
        register(&mut streams, id);
    }
    // Each removal frees a table slot for one more registration, and reserves 64 tombstone rows.
    for (idx, id) in (3..=6_u64).enumerate() {
        queue_set_distribution(&mut streams, &accruals, 0, 1, id, Address::new_id(300 + id))
            .unwrap();
        queue_remove_stream(&mut streams, &accruals, 0, 1, id).unwrap();
        register(&mut streams, 11 + idx as u64);
    }
    for op in [PendingWriteOp::SetWeightRecords, PendingWriteOp::StepWeightRecords] {
        queue_weight_records(
            &mut streams,
            &accruals,
            0,
            1,
            op,
            &[WeightRecordUpdate { id: 14, weight: constant_weight(0) }],
        )
        .unwrap();
    }
    for id in 7..=10_u64 {
        queue_set_distribution(&mut streams, &accruals, 0, 1, id, Address::new_id(300 + id))
            .unwrap();
    }
    assert_eq!(MAX_PENDING_WRITES, streams.pending_writes.len());
    structure(&streams).unwrap();

    // Stream 11 is live with its writer slot free, so only the bound stands in the way.
    let error = queue_set_distribution(&mut streams, &accruals, 0, 1, 11, Address::new_id(311))
        .unwrap_err();
    assert_eq!(
        format!(
            "pending write count {} exceeds maximum {MAX_PENDING_WRITES}",
            MAX_PENDING_WRITES + 1
        ),
        error.to_string()
    );
    assert_eq!(MAX_PENDING_WRITES, streams.pending_writes.len());
}

#[test]
fn structure_rejects_unordered_pending_writes() {
    let (mut streams, _) = base_state();
    streams.pending_writes = vec![
        PendingWrite {
            id: Some(1),
            op: PendingWriteOp::RegisterStream,
            payload: fvm_ipld_encoding::RawBytes::new(Vec::new()),
            effective_epoch: 2,
        },
        PendingWrite {
            id: Some(2),
            op: PendingWriteOp::RegisterStream,
            payload: fvm_ipld_encoding::RawBytes::new(Vec::new()),
            effective_epoch: 1,
        },
    ];

    let error = structure(&streams).unwrap_err();
    assert_eq!("pending writes are not ordered", error.to_string());
}

#[test]
fn structure_rejects_an_undecodable_pending_payload() {
    let (mut streams, _) = base_state();
    streams.pending_writes.push(PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: fvm_ipld_encoding::RawBytes::new(vec![0xff]),
        effective_epoch: 1,
    });

    assert!(structure(&streams).is_err());
}

#[test]
fn applies_same_epoch_lifecycle_writes_in_operation_order() {
    let (mut streams, mut accruals) = base_state();
    let new_stream = stream(3, 0, Some(explicit(203, shares(&[(103, DENOM)]))));
    queue_register_stream(&mut streams, &accruals, 0, 10, new_stream, 10).unwrap();
    queue_remove_stream(&mut streams, &accruals, 0, 10, 3).unwrap();

    apply_due_writes(&mut streams, &mut accruals, 9);
    assert_eq!(2, streams.streams.len());
    assert_eq!(10, next_epoch(&streams));

    apply_due_writes(&mut streams, &mut accruals, 10);
    assert_eq!(vec![1, 2], streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>());
    assert_eq!(vec![2], accruals.iter().map(|row| row.id).collect::<Vec<_>>());
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
}

#[test]
fn drops_a_weight_batch_stranded_by_cancelled_registration() {
    let (mut streams, mut accruals) = base_state();
    queue_register_stream(
        &mut streams,
        &accruals,
        0,
        10,
        stream(3, 0, Some(explicit(203, shares(&[(103, DENOM)])))),
        10,
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[
            WeightRecordUpdate { id: 1, weight: constant_weight(pct(50)) },
            WeightRecordUpdate { id: 3, weight: constant_weight(pct(10)) },
        ],
    )
    .unwrap();
    queue_set_distribution(&mut streams, &accruals, 0, 10, 2, Address::new_id(999)).unwrap();
    assert!(cancel(&mut streams, Some(3), PendingWriteOp::RegisterStream).unwrap().is_some());

    let result = apply_due_writes(&mut streams, &mut accruals, 10);
    assert_eq!(1, result.dropped.len());
    assert_eq!(PendingWriteOp::SetWeightRecords, result.dropped[0].op);
    assert_eq!(
        vec![PendingWriteOp::SetDistribution],
        result.applied.iter().map(|write| write.op).collect::<Vec<_>>()
    );
    assert_eq!(vec![1, 2], streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>());
    assert_eq!(pct(60), streams.streams[0].weight.v_start);
    assert_eq!(Address::new_id(999), streams.streams[1].distribution.as_ref().unwrap().writer);
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));

    let allocation = allocate(&streams.streams, 10, &TokenAmount::from_atto(100));
    assert_eq!(TokenAmount::from_atto(60), allocation.miner);
    assert_eq!(TokenAmount::from_atto(20), allocation.portions[0].1);
    assert_eq!(TokenAmount::from_atto(20), allocation.burn);
}

#[test]
fn admits_a_timelocked_weight_repair_and_drops_non_repairing_due_writes() {
    let (mut streams, mut accruals) = base_state();
    queue_set_distribution(&mut streams, &accruals, 0, 2, 2, Address::new_id(999)).unwrap();

    streams.streams[0].weight = constant_weight(pct(90));
    assert!(validate_streams_state(&streams, &accruals, 0).is_err());
    assert!(queue_register_stream(&mut streams, &accruals, 0, 2, stream(3, 0, None), 2,).is_err());
    assert!(
        queue_weight_records(
            &mut streams,
            &accruals,
            0,
            2,
            PendingWriteOp::StepWeightRecords,
            &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(70)) }],
        )
        .is_err()
    );

    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        2,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(70)) }],
    )
    .unwrap();

    let before = streams.clone();
    assert_eq!(ApplyResult::default(), apply_due_writes(&mut streams, &mut accruals, 1));
    assert_eq!(before, streams);
    assert_eq!(2, next_epoch(&streams));

    let result = apply_due_writes(&mut streams, &mut accruals, 2);
    assert_eq!(
        vec![PendingWriteOp::SetDistribution],
        result.dropped.iter().map(|write| write.op).collect::<Vec<_>>()
    );
    assert_eq!(
        vec![PendingWriteOp::SetWeightRecords],
        result.applied.iter().map(|write| write.op).collect::<Vec<_>>()
    );
    assert_eq!(pct(70), streams.streams[0].weight.v_start);
    assert_eq!(Address::new_id(200), streams.streams[1].distribution.as_ref().unwrap().writer);
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
    validate_streams_state(&streams, &accruals, 2).unwrap();
}

#[test]
fn applies_a_due_weight_repair_from_a_malformed_record() {
    let (mut streams, mut accruals) = base_state();
    streams.streams[0].weight = constant_weight(DENOM + 1);

    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        2,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(60)) }],
    )
    .unwrap();

    let result = apply_due_writes(&mut streams, &mut accruals, 2);

    assert_eq!(
        vec![PendingWriteOp::SetWeightRecords],
        result.applied.iter().map(|write| write.op).collect::<Vec<_>>()
    );
    assert!(result.dropped.is_empty());
    assert_eq!(pct(60), streams.streams[0].weight.v_start);
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
    validate_streams_state(&streams, &accruals, 2).unwrap();
}

#[test]
fn drops_two_dependents_stranded_by_one_cancelled_registration() {
    let (mut streams, mut accruals) = base_state();
    queue_register_stream(
        &mut streams,
        &accruals,
        0,
        10,
        stream(3, 0, Some(explicit(203, shares(&[(103, DENOM)])))),
        10,
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 3, weight: constant_weight(pct(10)) }],
    )
    .unwrap();
    queue_set_distribution(&mut streams, &accruals, 0, 10, 3, Address::new_id(303)).unwrap();
    assert!(cancel(&mut streams, Some(3), PendingWriteOp::RegisterStream).unwrap().is_some());

    let result = apply_due_writes(&mut streams, &mut accruals, 10);
    assert_eq!(
        vec![PendingWriteOp::SetWeightRecords, PendingWriteOp::SetDistribution],
        result.dropped.iter().map(|write| write.op).collect::<Vec<_>>()
    );
    assert!(streams.pending_writes.is_empty());
    assert_eq!(vec![1, 2], streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>());
}

#[test]
fn new_admission_can_revive_a_previously_stranded_call() {
    let (mut streams, mut accruals) = base_state();
    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(50)) }],
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        20,
        PendingWriteOp::StepWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(50)) }],
    )
    .unwrap();
    assert!(cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some());
    let stranded = apply_due_writes(&mut streams.clone(), &mut accruals.clone(), 20);
    assert_eq!(1, stranded.dropped.len());

    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(50)) }],
    )
    .unwrap();
    let result = apply_due_writes(&mut streams, &mut accruals, 20);
    assert!(result.dropped.is_empty());
    assert_eq!(pct(50), streams.streams[0].weight.v_start);
    assert_eq!(pct(50), streams.streams[1].weight.v_start);
}

// Application validates a write from its own effective epoch, exactly as admission projected
// it, so a null round at that epoch cannot revive a stranded write and drop one admitted on
// its projected removal.
#[test]
fn null_round_at_effective_epoch_cannot_revive_a_stranded_call() {
    let (mut streams, mut accruals) = base_state();
    // Stream 1 ramps from 60% at epoch 10 to 40% at epoch 12.
    streams.streams[0].weight = WeightRecord {
        v_start: pct(60),
        slope: -(pct(10) as i64),
        t_start: 10,
        floor: pct(40),
        cap: pct(60),
    };
    queue_remove_stream(&mut streams, &accruals, 3, 7, 1).unwrap();
    // Valid at 10 only because the removal precedes it; valid on its own from 11.
    queue_weight_records(
        &mut streams,
        &accruals,
        3,
        7,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(45)) }],
    )
    .unwrap();
    assert!(cancel(&mut streams, Some(1), PendingWriteOp::RemoveStream).unwrap().is_some());
    // Admitted at 12 on the projection that the stranded weight write drops: 40 + 20 + 20.
    queue_register_stream(
        &mut streams,
        &accruals,
        9,
        3,
        stream(3, pct(20), Some(explicit(203, shares(&[(103, DENOM)])))),
        12,
    )
    .unwrap();

    let drop_then_register =
        |streams: &mut StreamsState, accruals: &mut Vec<StreamAccrual>, first_award: ChainEpoch| {
            let result = apply_due_writes(streams, accruals, first_award);
            assert!(result.applied.is_empty());
            assert_eq!(
                vec![PendingWriteOp::SetWeightRecords],
                result.dropped.iter().map(|write| write.op).collect::<Vec<_>>()
            );
            let result = apply_due_writes(streams, accruals, 12);
            assert!(result.dropped.is_empty());
            assert_eq!(
                vec![PendingWriteOp::RegisterStream],
                result.applied.iter().map(|write| write.op).collect::<Vec<_>>()
            );
            assert_eq!(
                vec![1, 2, 3],
                streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>()
            );
            assert_eq!(pct(20), streams.streams[1].weight.v_start);
        };
    let (mut null_round_streams, mut null_round_accruals) = (streams.clone(), accruals.clone());
    // Award at 10, the stranded write's effective epoch.
    drop_then_register(&mut streams, &mut accruals, 10);
    // Null round at 10; the first award after it reaches the same schedule.
    drop_then_register(&mut null_round_streams, &mut null_round_accruals, 11);
    assert_eq!(streams, null_round_streams);
}

#[test]
fn drops_a_stranded_gate_batch_and_accepts_the_next_absolute_level() {
    let (mut streams, mut accruals) = base_state();
    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(40)) }],
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        10,
        PendingWriteOp::StepWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(55)) }],
    )
    .unwrap();
    assert!(cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some());

    let result = apply_due_writes(&mut streams, &mut accruals, 10);
    assert_eq!(1, result.dropped.len());
    assert_eq!(PendingWriteOp::StepWeightRecords, result.dropped[0].op);
    assert_eq!(pct(20), streams.streams[1].weight.v_start);

    queue_weight_records(
        &mut streams,
        &accruals,
        10,
        0,
        PendingWriteOp::StepWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(30)) }],
    )
    .unwrap();
    let result = apply_due_writes(&mut streams, &mut accruals, 10);
    assert!(result.dropped.is_empty());
    assert_eq!(pct(30), streams.streams[1].weight.v_start);
}

#[test]
fn repairs_a_dropped_terminal_gate_with_a_discretionary_write() {
    let (mut streams, mut accruals) = base_state();
    streams.streams[0].weight = constant_weight(pct(55));
    streams.streams[1].weight = constant_weight(pct(45));
    let steps = 8;
    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(50)) }],
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        10,
        PendingWriteOp::StepWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(50)) }],
    )
    .unwrap();
    assert!(cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some());

    let result = apply_due_writes(&mut streams, &mut accruals, 10);
    assert_eq!(1, result.dropped.len());
    assert_eq!(pct(45), streams.streams[1].weight.v_start);
    assert_eq!(8, steps);

    queue_weight_records(
        &mut streams,
        &accruals,
        10,
        0,
        PendingWriteOp::SetWeightRecords,
        &[
            WeightRecordUpdate { id: 1, weight: constant_weight(pct(50)) },
            WeightRecordUpdate { id: 2, weight: constant_weight(pct(50)) },
        ],
    )
    .unwrap();
    let result = apply_due_writes(&mut streams, &mut accruals, 10);
    assert!(result.dropped.is_empty());
    assert_eq!(pct(50), streams.streams[0].weight.v_start);
    assert_eq!(pct(50), streams.streams[1].weight.v_start);
    assert_eq!(8, steps);
}

#[test]
fn preserves_queue_position_for_equal_epoch_calls() {
    let (mut streams, mut accruals) = base_state();
    queue_set_distribution(&mut streams, &accruals, 0, 10, 2, Address::new_id(999)).unwrap();
    queue_remove_stream(&mut streams, &accruals, 0, 10, 2).unwrap();

    let result = apply_due_writes(&mut streams, &mut accruals, 10);
    assert!(result.dropped.is_empty());
    assert_eq!(vec![1], streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>());
}

#[test]
fn removal_settles_into_a_claimable_tombstone() {
    let (mut streams, mut accruals) = base_state();
    let distribution = streams.streams[1].distribution.as_mut().unwrap();
    distribution.shares = shares(&[(101, DENOM / 2), (102, DENOM - DENOM / 2)]);
    distribution.payable.add(Address::new_id(102), TokenAmount::from_atto(3));
    distribution.claimed_period.add(Address::new_id(101), TokenAmount::from_atto(2));
    accruals[0].amount = TokenAmount::from_atto(11);

    queue_remove_stream(&mut streams, &accruals, 0, 1, 2).unwrap();
    let result = apply_due_writes(&mut streams, &mut accruals, 1);
    assert_eq!(TokenAmount::from_atto(1), result.fold_dust);
    assert_eq!(vec![1], streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>());
    assert!(accruals.is_empty());
    assert_eq!(TokenAmount::from_atto(3), amount(&streams.tombstones[0].payable, 101));
    assert_eq!(TokenAmount::from_atto(8), amount(&streams.tombstones[0].payable, 102));

    let claimed =
        claim(&mut streams, &accruals, 2, &[Address::new_id(101), Address::new_id(102)]).unwrap();
    assert_eq!(vec![TokenAmount::from_atto(3), TokenAmount::from_atto(8)], claimed);
    assert!(streams.tombstones.is_empty());
}

#[test]
fn writer_replacement_settles_before_repointing() {
    let (mut streams, mut accruals) = base_state();
    streams.streams[1].distribution.as_mut().unwrap().shares =
        shares(&[(101, DENOM / 2), (102, DENOM - DENOM / 2)]);
    accruals[0].amount = TokenAmount::from_atto(5);

    queue_set_distribution(&mut streams, &accruals, 0, 1, 2, Address::new_id(999)).unwrap();
    let result = apply_due_writes(&mut streams, &mut accruals, 1);
    let distribution = streams.streams[1].distribution.as_ref().unwrap();
    assert_eq!(TokenAmount::from_atto(1), result.fold_dust);
    assert_eq!(Address::new_id(999), distribution.writer);
    assert_eq!(TokenAmount::from_atto(2), amount(&distribution.payable, 101));
    assert_eq!(TokenAmount::from_atto(2), amount(&distribution.payable, 102));
    assert_eq!(TokenAmount::zero(), accruals[0].amount);
}

#[test]
fn conserves_explicit_value_across_claims_folds_and_removal() {
    let third = DENOM / 3;
    let mut streams = StreamsState {
        streams: vec![stream(
            2,
            pct(20),
            Some(explicit(200, shares(&[(101, third), (102, third), (103, DENOM - 2 * third)]))),
        )],
        ..Default::default()
    };
    let mut accruals = vec![StreamAccrual { id: 2, amount: TokenAmount::from_atto(10) }];
    let mut gross = TokenAmount::from_atto(10);
    let mut paid = TokenAmount::zero();
    let mut burned = TokenAmount::zero();
    assert_explicit_conserved(&gross, &paid, &burned, &streams, &accruals);

    let result = claim(&mut streams, &accruals, 2, &[Address::new_id(101)]).unwrap();
    paid += &result[0];
    assert_explicit_conserved(&gross, &paid, &burned, &streams, &accruals);

    burned += set_shares(&mut streams, &mut accruals, 2, shares(&[(104, DENOM)])).unwrap();
    assert_explicit_conserved(&gross, &paid, &burned, &streams, &accruals);

    accruals[0].amount += TokenAmount::from_atto(5);
    gross += TokenAmount::from_atto(5);
    let result = claim(&mut streams, &accruals, 2, &[Address::new_id(104)]).unwrap();
    paid += &result[0];
    assert_explicit_conserved(&gross, &paid, &burned, &streams, &accruals);
    queue_remove_stream(&mut streams, &accruals, 0, 1, 2).unwrap();
    burned += apply_due_writes(&mut streams, &mut accruals, 1).fold_dust;
    assert_explicit_conserved(&gross, &paid, &burned, &streams, &accruals);

    let result =
        claim(&mut streams, &accruals, 2, &[Address::new_id(102), Address::new_id(103)]).unwrap();
    paid += result.iter().fold(TokenAmount::zero(), |total, amount| total + amount);
    assert!(streams.tombstones.is_empty());
    assert_explicit_conserved(&gross, &paid, &burned, &streams, &accruals);
}

#[test]
fn projects_due_writes_without_mutating_stored_state() {
    let (mut streams, accruals) = base_state();
    queue_weight_records(
        &mut streams,
        &accruals,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(70)) }],
    )
    .unwrap();

    let mut projected = streams.clone();
    let result = apply_due_writes(&mut projected, &mut accruals.clone(), 10);
    assert_eq!(pct(70), projected.streams[0].weight.v_start);
    assert!(projected.pending_writes.is_empty());
    assert_eq!(TokenAmount::zero(), result.fold_dust);

    assert_eq!(pct(60), streams.streams[0].weight.v_start);
    assert_eq!(1, streams.pending_writes.len());
    assert_eq!(10, next_epoch(&streams));
}

#[test]
fn randomized_conservation_covers_the_full_operation_mix_and_drops() {
    let (mut streams, mut accruals) = base_state();
    let mut supply = SupplyTracker::default();
    let mut random = 0x05ee_df02_u64;
    let mut epoch = 0;
    let mut next_id = 4;
    let mut covered = [false; 9];

    queue_register_stream(
        &mut streams,
        &accruals,
        epoch,
        2,
        stream(3, 0, Some(explicit(203, shares(&[(103, DENOM)])))),
        epoch + 2,
    )
    .unwrap();
    covered[3] = true;
    queue_weight_records(
        &mut streams,
        &accruals,
        epoch,
        2,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 3, weight: constant_weight(pct(10)) }],
    )
    .unwrap();
    covered[6] = true;
    queue_set_distribution(&mut streams, &accruals, epoch, 2, 3, Address::new_id(303)).unwrap();
    covered[5] = true;
    assert!(cancel(&mut streams, Some(3), PendingWriteOp::RegisterStream).unwrap().is_some());
    covered[8] = true;
    epoch += 2;
    let result = apply_due_writes(&mut streams, &mut accruals, epoch);
    assert_eq!(2, result.dropped.len());
    supply.burn_dust(result.fold_dust);
    let mut dropped = result.dropped.len();

    for _ in 0..512 {
        epoch += 1;
        let result = apply_due_writes(&mut streams, &mut accruals, epoch);
        dropped += result.dropped.len();
        supply.burn_dust(result.fold_dust);

        let previous =
            (supply.total_minted.clone(), supply.total_burn.clone(), supply.total_explicit.clone());
        match random_u64(&mut random) % covered.len() as u64 {
            0 => {
                let reward = TokenAmount::from_atto(random_u64(&mut random) % 1_000 + 1);
                supply.award(&streams, &mut accruals, epoch, reward);
                covered[0] = true;
            }
            1 => {
                let mut targets = Vec::new();
                for stream in &streams.streams {
                    if let Some(distribution) = &stream.distribution {
                        targets.extend(
                            distribution.shares.iter().map(|share| (stream.id, share.recipient)),
                        );
                        targets.extend(
                            distribution.payable.iter().map(|row| (stream.id, row.recipient)),
                        );
                    }
                }
                for tombstone in &streams.tombstones {
                    targets
                        .extend(tombstone.payable.iter().map(|row| (tombstone.id, row.recipient)));
                }
                let (id, wallet) =
                    if targets.is_empty() || random_u64(&mut random).is_multiple_of(4) {
                        (999_999, Address::new_id(999_999))
                    } else {
                        targets[random_u64(&mut random) as usize % targets.len()]
                    };
                let result =
                    claim(&mut streams, &accruals, id, &[wallet, wallet, Address::new_id(999_998)])
                        .unwrap();
                supply.pay_claim(&result);
                covered[1] = true;
            }
            2 => {
                let ids: Vec<_> = streams
                    .streams
                    .iter()
                    .filter(|stream| stream.distribution.is_some())
                    .map(|stream| stream.id)
                    .collect();
                if !ids.is_empty() {
                    let id = ids[random_u64(&mut random) as usize % ids.len()];
                    let wallet = 10_000 + random_u64(&mut random) % 32;
                    let new_shares = if random_u64(&mut random) & 1 == 0 {
                        shares(&[(wallet, DENOM)])
                    } else {
                        let first = random_u64(&mut random) % (DENOM - 1) + 1;
                        shares(&[(wallet, first), (wallet + 100, DENOM - first)])
                    };
                    // A rejected fold leaves the ledger unspecified, so keep the copy only when
                    // it was accepted.
                    let mut next_streams = streams.clone();
                    let mut next_accruals = accruals.clone();
                    if let Ok(dust) =
                        set_shares(&mut next_streams, &mut next_accruals, id, new_shares)
                    {
                        streams = next_streams;
                        accruals = next_accruals;
                        supply.burn_dust(dust);
                        covered[2] = true;
                    }
                }
            }
            3 => {
                let id = next_id;
                let new_stream =
                    stream(id, 0, Some(explicit(20_000 + id, shares(&[(30_000 + id, DENOM)]))));
                if queue_register_stream(&mut streams, &accruals, epoch, 2, new_stream, epoch + 2)
                    .is_ok()
                {
                    next_id += 1;
                    covered[3] = true;
                }
            }
            4 => {
                let ids: Vec<_> = streams
                    .streams
                    .iter()
                    .filter(|stream| stream.distribution.is_some())
                    .map(|stream| stream.id)
                    .collect();
                if !ids.is_empty() {
                    let id = ids[random_u64(&mut random) as usize % ids.len()];
                    if queue_remove_stream(&mut streams, &accruals, epoch, 2, id).is_ok() {
                        covered[4] = true;
                    }
                }
            }
            5 => {
                let ids: Vec<_> = streams
                    .streams
                    .iter()
                    .filter(|stream| stream.distribution.is_some())
                    .map(|stream| stream.id)
                    .collect();
                if !ids.is_empty() {
                    let id = ids[random_u64(&mut random) as usize % ids.len()];
                    let writer = Address::new_id(40_000 + random_u64(&mut random) % 32);
                    if queue_set_distribution(&mut streams, &accruals, epoch, 2, id, writer).is_ok()
                    {
                        covered[5] = true;
                    }
                }
            }
            6 | 7 => {
                let op = if random_u64(&mut random) & 1 == 0 {
                    PendingWriteOp::SetWeightRecords
                } else {
                    PendingWriteOp::StepWeightRecords
                };
                let update = WeightRecordUpdate {
                    id: 1,
                    weight: streams
                        .streams
                        .iter()
                        .find(|stream| stream.id == 1)
                        .unwrap()
                        .weight
                        .clone(),
                };
                if queue_weight_records(&mut streams, &accruals, epoch, 2, op, &[update]).is_ok() {
                    covered[if op == PendingWriteOp::SetWeightRecords { 6 } else { 7 }] = true;
                }
            }
            8 => {
                let cancellable: Vec<_> = streams
                    .pending_writes
                    .iter()
                    .filter(|write| write.op != PendingWriteOp::StepWeightRecords)
                    .map(|write| (write.id, write.op))
                    .collect();
                if !cancellable.is_empty() {
                    let (id, op) =
                        cancellable[random_u64(&mut random) as usize % cancellable.len()];
                    if cancel(&mut streams, id, op).unwrap().is_some() {
                        covered[8] = true;
                    }
                }
            }
            _ => unreachable!(),
        }

        assert!(supply.total_minted >= previous.0);
        assert!(supply.total_burn >= previous.1);
        assert!(supply.total_explicit >= previous.2);
        supply.assert_invariants(&streams, &accruals);
        validate_streams_state(&streams, &accruals, epoch).unwrap();
    }

    let result = apply_due_writes(&mut streams, &mut accruals, epoch + 2);
    dropped += result.dropped.len();
    supply.burn_dust(result.fold_dust);
    supply.assert_invariants(&streams, &accruals);
    assert!(covered.iter().all(|covered| *covered), "missing operation coverage: {covered:?}");
    assert!(dropped >= 2);
}
