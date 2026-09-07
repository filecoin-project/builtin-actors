use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_shared::address::Address;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::econ::TokenAmount;
use hex_literal::hex;

use super::*;
use crate::streams::invariants::structure;
use crate::streams::weights::compute_weight;

fn next_epoch(streams: &StreamsState) -> ChainEpoch {
    streams.pending_writes.first().map_or(EPOCH_UNDEFINED, |write| write.effective_epoch)
}

/// A seeded, deterministic pseudo-random generator for the randomized test below. Callers take
/// small moduli of the result, so it returns the well-mixed high bits of the state.
fn random_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *state >> 33
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
fn rejects_out_of_band_anchors_on_every_weight_queue_operation() {
    let invalid = [
        ("below floor", WeightRecord { v_start: 10, slope: 1, t_start: 0, floor: 20, cap: 30 }),
        ("exceeds cap", WeightRecord { v_start: 30, slope: -1, t_start: 0, floor: 10, cap: 20 }),
    ];

    for op in [PendingWriteOp::SetWeightRecords, PendingWriteOp::StepWeightRecords] {
        for (message, weight) in &invalid {
            let mut streams = base_state();
            let before = streams.clone();
            let error = queue_weight_records(
                &mut streams,
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
        let mut streams = base_state();
        let before = streams.clone();
        let error = queue_register_stream(
            &mut streams,
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
fn burns_the_sentinel_share_of_every_award() {
    let mut streams = StreamsState {
        streams: vec![
            stream(1, pct(60), None),
            stream(2, pct(20), Some(explicit(200, shares(&[(101, pct(25)), (102, pct(75))])))),
        ],
        ..Default::default()
    };
    let reward = TokenAmount::from_atto(100);

    let full = allocate(&streams.streams, 0, &reward);
    assert_eq!(TokenAmount::from_atto(60), full.miner);
    assert_eq!(TokenAmount::from_atto(5), payout(&full, 2, 101));
    assert_eq!(TokenAmount::from_atto(15), payout(&full, 2, 102));
    assert_eq!(TokenAmount::from_atto(20), full.burn);

    // Half the map is the sentinel, so half the stream's portion burns instead of being paid.
    set_shares(
        &mut streams,
        2,
        shares(&[(99, pct(25)), (101, pct(25)), (99, pct(25)), (102, pct(25))]),
    )
    .unwrap();
    assert_eq!(
        shares(&[(101, pct(25)), (102, pct(25))]),
        streams.streams[1].distribution.as_ref().unwrap().shares
    );

    let partial = allocate(&streams.streams, 0, &reward);
    assert_eq!(TokenAmount::from_atto(60), partial.miner);
    assert_eq!(TokenAmount::from_atto(5), payout(&partial, 2, 101));
    assert_eq!(TokenAmount::from_atto(5), payout(&partial, 2, 102));
    assert_eq!(TokenAmount::from_atto(30), partial.burn);
    assert_eq!(reward, &partial.miner + &paid(&partial) + &partial.burn);
    assert!(partial.payouts.iter().all(|payout| payout.recipient != BURNT_FUNDS_ACTOR_ADDR));
}

#[test]
fn rejects_a_new_call_that_strands_an_existing_call() {
    let mut streams = base_state();

    queue_weight_records(
        &mut streams,
        0,
        30,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(40)) }],
    )
    .unwrap();
    assert_eq!(30, next_epoch(&streams));

    let error = queue_weight_records(
        &mut streams,
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
    let mut streams = base_state();
    queue_weight_records(
        &mut streams,
        0,
        2,
        PendingWriteOp::StepWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(30)) }],
    )
    .unwrap();

    let before = streams.clone();
    let error = queue_remove_stream(&mut streams, 0, 1, 2).unwrap_err();
    assert!(error.to_string().contains("invalidates an existing pending call"));
    assert_eq!(before, streams);

    let result = apply_due_writes(&mut streams, 2);
    assert_eq!(
        vec![PendingWriteOp::StepWeightRecords],
        result.applied.iter().map(|w| w.op).collect::<Vec<_>>()
    );
    assert!(result.dropped.is_empty());
    assert_eq!(pct(30), streams.streams[1].weight.v_start);

    queue_remove_stream(&mut streams, 2, 1, 2).unwrap();
    assert_eq!(3, next_epoch(&streams));
}

#[test]
fn rejects_a_schedule_that_depends_on_a_later_call() {
    let mut streams = base_state();

    queue_weight_records(
        &mut streams,
        0,
        20,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(20)) }],
    )
    .unwrap();
    let error = queue_weight_records(
        &mut streams,
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

        let mut streams = base_state();
        let error = queue_weight_records(
            &mut streams,
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
    let mut streams = base_state();
    let updates = [
        WeightRecordUpdate { id: 1, weight: constant_weight(pct(65)) },
        WeightRecordUpdate { id: 2, weight: constant_weight(pct(35)) },
    ];

    assert_eq!(
        17,
        queue_weight_records(&mut streams, 10, 7, PendingWriteOp::SetWeightRecords, &updates,)
            .unwrap()
            .effective_epoch
    );
    assert_eq!(17, next_epoch(&streams));
    assert_eq!(1, streams.pending_writes.len());
    assert_eq!(None, streams.pending_writes[0].id);
    let payload: WeightRecordsPayload = streams.pending_writes[0].payload.deserialize().unwrap();
    assert_eq!(updates, payload.updates.as_slice());
    assert!(
        queue_weight_records(&mut streams, 10, 7, PendingWriteOp::SetWeightRecords, &updates[..1],)
            .is_err()
    );

    assert!(cancel(&mut streams, Some(999), PendingWriteOp::SetWeightRecords).is_err());
    assert_eq!(17, next_epoch(&streams));
    assert!(cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some());
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
    assert!(cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_none());
    assert!(cancel(&mut streams, None, PendingWriteOp::RemoveStream).is_err());

    queue_weight_records(&mut streams, 20, 7, PendingWriteOp::SetWeightRecords, &updates[1..])
        .unwrap();
    assert_eq!(27, next_epoch(&streams));
    assert!(cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some());
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));

    queue_weight_records(&mut streams, 10, 7, PendingWriteOp::StepWeightRecords, &updates[..1])
        .unwrap();
    assert!(cancel(&mut streams, None, PendingWriteOp::StepWeightRecords).is_err());
    let result = apply_due_writes(&mut streams, 17);
    let removed = cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap();
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
            17,
            0,
            PendingWriteOp::SetWeightRecords,
            &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(30)) }],
        )
        .unwrap()
        .effective_epoch
    );
    assert_eq!(17, next_epoch(&streams));
    apply_due_writes(&mut streams, 17);
    assert_eq!(pct(30), streams.streams[1].weight.v_start);
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
}

#[test]
fn apply_and_cancel_surfaces_calls_dropped_before_cancellation() {
    let mut streams = base_state();
    queue_register_stream(
        &mut streams,
        0,
        10,
        stream(3, 0, Some(explicit(203, shares(&[(103, DENOM)])))),
        10,
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 3, weight: constant_weight(pct(10)) }],
    )
    .unwrap();
    assert!(cancel(&mut streams, Some(3), PendingWriteOp::RegisterStream).unwrap().is_some());

    let result = apply_due_writes(&mut streams, 10);
    let removed = cancel(&mut streams, Some(99), PendingWriteOp::RemoveStream).unwrap();
    assert_eq!(1, result.dropped.len());
    assert_eq!(PendingWriteOp::SetWeightRecords, result.dropped[0].op);
    assert!(removed.is_none());
    assert!(streams.pending_writes.is_empty());
}

#[test]
fn enforces_registration_bounds_and_id_availability() {
    let mut streams = base_state();
    let new_stream =
        stream(3, 0, Some(explicit(201, shares(&[(103, DENOM / 2), (102, DENOM - DENOM / 2)]))));

    assert!(queue_register_stream(&mut streams, 10, 7, new_stream.clone(), 16).is_err());
    assert_eq!(
        17,
        queue_register_stream(&mut streams, 10, 7, new_stream, 17).unwrap().effective_epoch
    );
    let payload: RegisterStreamPayload = streams.pending_writes[0].payload.deserialize().unwrap();
    assert_eq!(
        vec![Address::new_id(102), Address::new_id(103)],
        payload.distribution.unwrap().shares.iter().map(|row| row.recipient).collect::<Vec<_>>()
    );
    assert!(
        queue_register_stream(
            &mut streams,
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
    assert!(
        queue_register_stream(
            &mut full,
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
    queue_register_stream(
        &mut seven,
        0,
        1,
        stream(8, 0, Some(explicit(208, shares(&[(108, DENOM)])))),
        1,
    )
    .unwrap();
    apply_due_writes(&mut seven, 1);
    assert_eq!(MAX_STREAMS, seven.streams.len());

    // An ID is taken while its stream is live, and free again once its removal has applied.
    let mut reused = StreamsState {
        streams: vec![stream(4, 0, Some(explicit(204, shares(&[(104, DENOM)]))))],
        ..Default::default()
    };
    let replacement = stream(4, 0, Some(explicit(214, shares(&[(114, DENOM)]))));
    assert!(queue_register_stream(&mut reused, 0, 1, replacement.clone(), 1).is_err());

    queue_remove_stream(&mut reused, 0, 1, 4).unwrap();
    apply_due_writes(&mut reused, 1);
    assert!(reused.streams.is_empty());
    assert_eq!(
        2,
        queue_register_stream(&mut reused, 1, 1, replacement, 2).unwrap().effective_epoch
    );
    apply_due_writes(&mut reused, 2);
    assert_eq!(
        vec![Address::new_id(114)],
        reused.streams[0]
            .distribution
            .as_ref()
            .unwrap()
            .shares
            .iter()
            .map(|row| row.recipient)
            .collect::<Vec<_>>()
    );
}

#[test]
fn rejects_invalid_timelocks_without_mutation() {
    let mut streams = base_state();
    let before = streams.clone();

    assert!(queue_remove_stream(&mut streams, 0, -1, 2).is_err());
    assert!(queue_remove_stream(&mut streams, i64::MAX, 1, 2).is_err());
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
    let register = |streams: &mut StreamsState, id: StreamId| {
        queue_register_stream(
            streams,
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
    // Each removal frees a table slot for one more registration.
    for (idx, id) in (3..=6_u64).enumerate() {
        queue_set_distribution(&mut streams, 0, 1, id, Address::new_id(300 + id)).unwrap();
        queue_remove_stream(&mut streams, 0, 1, id).unwrap();
        register(&mut streams, 11 + idx as u64);
    }
    for op in [PendingWriteOp::SetWeightRecords, PendingWriteOp::StepWeightRecords] {
        queue_weight_records(
            &mut streams,
            0,
            1,
            op,
            &[WeightRecordUpdate { id: 14, weight: constant_weight(0) }],
        )
        .unwrap();
    }
    for id in 7..=10_u64 {
        queue_set_distribution(&mut streams, 0, 1, id, Address::new_id(300 + id)).unwrap();
    }
    assert_eq!(MAX_PENDING_WRITES, streams.pending_writes.len());
    structure(&streams).unwrap();

    // Stream 11 is live with its writer slot free, so only the bound stands in the way.
    let error = queue_set_distribution(&mut streams, 0, 1, 11, Address::new_id(311)).unwrap_err();
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
    let mut streams = base_state();
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
    let mut streams = base_state();
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
    let mut streams = base_state();
    let new_stream = stream(3, 0, Some(explicit(203, shares(&[(103, DENOM)]))));
    queue_register_stream(&mut streams, 0, 10, new_stream, 10).unwrap();
    queue_remove_stream(&mut streams, 0, 10, 3).unwrap();

    apply_due_writes(&mut streams, 9);
    assert_eq!(2, streams.streams.len());
    assert_eq!(10, next_epoch(&streams));

    apply_due_writes(&mut streams, 10);
    assert_eq!(vec![1, 2], streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>());
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
}

#[test]
fn drops_a_weight_batch_stranded_by_cancelled_registration() {
    let mut streams = base_state();
    queue_register_stream(
        &mut streams,
        0,
        10,
        stream(3, 0, Some(explicit(203, shares(&[(103, DENOM)])))),
        10,
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[
            WeightRecordUpdate { id: 1, weight: constant_weight(pct(50)) },
            WeightRecordUpdate { id: 3, weight: constant_weight(pct(10)) },
        ],
    )
    .unwrap();
    queue_set_distribution(&mut streams, 0, 10, 2, Address::new_id(999)).unwrap();
    assert!(cancel(&mut streams, Some(3), PendingWriteOp::RegisterStream).unwrap().is_some());

    let result = apply_due_writes(&mut streams, 10);
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
    assert_eq!(TokenAmount::from_atto(20), payout(&allocation, 2, 101));
    assert_eq!(TokenAmount::from_atto(20), allocation.burn);
}

#[test]
fn admits_a_timelocked_weight_repair_and_drops_non_repairing_due_writes() {
    let mut streams = base_state();
    queue_set_distribution(&mut streams, 0, 2, 2, Address::new_id(999)).unwrap();

    streams.streams[0].weight = constant_weight(pct(90));
    assert!(validate_streams_state(&streams, 0).is_err());
    assert!(queue_register_stream(&mut streams, 0, 2, stream(3, 0, None), 2,).is_err());
    assert!(
        queue_weight_records(
            &mut streams,
            0,
            2,
            PendingWriteOp::StepWeightRecords,
            &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(70)) }],
        )
        .is_err()
    );

    queue_weight_records(
        &mut streams,
        0,
        2,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(70)) }],
    )
    .unwrap();

    let before = streams.clone();
    assert_eq!(ApplyResult::default(), apply_due_writes(&mut streams, 1));
    assert_eq!(before, streams);
    assert_eq!(2, next_epoch(&streams));

    let result = apply_due_writes(&mut streams, 2);
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
    validate_streams_state(&streams, 2).unwrap();
}

#[test]
fn applies_a_due_weight_repair_from_a_malformed_record() {
    let mut streams = base_state();
    streams.streams[0].weight = constant_weight(DENOM + 1);

    queue_weight_records(
        &mut streams,
        0,
        2,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(60)) }],
    )
    .unwrap();

    let result = apply_due_writes(&mut streams, 2);

    assert_eq!(
        vec![PendingWriteOp::SetWeightRecords],
        result.applied.iter().map(|write| write.op).collect::<Vec<_>>()
    );
    assert!(result.dropped.is_empty());
    assert_eq!(pct(60), streams.streams[0].weight.v_start);
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
    validate_streams_state(&streams, 2).unwrap();
}

#[test]
fn drops_two_dependents_stranded_by_one_cancelled_registration() {
    let mut streams = base_state();
    queue_register_stream(
        &mut streams,
        0,
        10,
        stream(3, 0, Some(explicit(203, shares(&[(103, DENOM)])))),
        10,
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 3, weight: constant_weight(pct(10)) }],
    )
    .unwrap();
    queue_set_distribution(&mut streams, 0, 10, 3, Address::new_id(303)).unwrap();
    assert!(cancel(&mut streams, Some(3), PendingWriteOp::RegisterStream).unwrap().is_some());

    let result = apply_due_writes(&mut streams, 10);
    assert_eq!(
        vec![PendingWriteOp::SetWeightRecords, PendingWriteOp::SetDistribution],
        result.dropped.iter().map(|write| write.op).collect::<Vec<_>>()
    );
    assert!(streams.pending_writes.is_empty());
    assert_eq!(vec![1, 2], streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>());
}

#[test]
fn new_admission_can_revive_a_previously_stranded_call() {
    let mut streams = base_state();
    queue_weight_records(
        &mut streams,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(50)) }],
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        0,
        20,
        PendingWriteOp::StepWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(50)) }],
    )
    .unwrap();
    assert!(cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some());
    let stranded = apply_due_writes(&mut streams.clone(), 20);
    assert_eq!(1, stranded.dropped.len());

    queue_weight_records(
        &mut streams,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(50)) }],
    )
    .unwrap();
    let result = apply_due_writes(&mut streams, 20);
    assert!(result.dropped.is_empty());
    assert_eq!(pct(50), streams.streams[0].weight.v_start);
    assert_eq!(pct(50), streams.streams[1].weight.v_start);
}

// Application validates a write from its own effective epoch, exactly as admission projected
// it, so a null round at that epoch cannot revive a stranded write and drop one admitted on
// its projected removal.
#[test]
fn null_round_at_effective_epoch_cannot_revive_a_stranded_call() {
    let mut streams = base_state();
    // Stream 1 ramps from 60% at epoch 10 to 40% at epoch 12.
    streams.streams[0].weight = WeightRecord {
        v_start: pct(60),
        slope: -(pct(10) as i64),
        t_start: 10,
        floor: pct(40),
        cap: pct(60),
    };
    queue_remove_stream(&mut streams, 3, 7, 1).unwrap();
    // Valid at 10 only because the removal precedes it; valid on its own from 11.
    queue_weight_records(
        &mut streams,
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
        9,
        3,
        stream(3, pct(20), Some(explicit(203, shares(&[(103, DENOM)])))),
        12,
    )
    .unwrap();

    let drop_then_register = |streams: &mut StreamsState, first_award: ChainEpoch| {
        let result = apply_due_writes(streams, first_award);
        assert!(result.applied.is_empty());
        assert_eq!(
            vec![PendingWriteOp::SetWeightRecords],
            result.dropped.iter().map(|write| write.op).collect::<Vec<_>>()
        );
        let result = apply_due_writes(streams, 12);
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
    let mut null_round_streams = streams.clone();
    // Award at 10, the stranded write's effective epoch.
    drop_then_register(&mut streams, 10);
    // Null round at 10; the first award after it reaches the same schedule.
    drop_then_register(&mut null_round_streams, 11);
    assert_eq!(streams, null_round_streams);
}

#[test]
fn drops_a_stranded_gate_batch_and_accepts_the_next_absolute_level() {
    let mut streams = base_state();
    queue_weight_records(
        &mut streams,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(40)) }],
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        0,
        10,
        PendingWriteOp::StepWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(55)) }],
    )
    .unwrap();
    assert!(cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some());

    let result = apply_due_writes(&mut streams, 10);
    assert_eq!(1, result.dropped.len());
    assert_eq!(PendingWriteOp::StepWeightRecords, result.dropped[0].op);
    assert_eq!(pct(20), streams.streams[1].weight.v_start);

    queue_weight_records(
        &mut streams,
        10,
        0,
        PendingWriteOp::StepWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(30)) }],
    )
    .unwrap();
    let result = apply_due_writes(&mut streams, 10);
    assert!(result.dropped.is_empty());
    assert_eq!(pct(30), streams.streams[1].weight.v_start);
}

#[test]
fn repairs_a_dropped_terminal_gate_with_a_discretionary_write() {
    let mut streams = base_state();
    streams.streams[0].weight = constant_weight(pct(55));
    streams.streams[1].weight = constant_weight(pct(45));
    let steps = 8;
    queue_weight_records(
        &mut streams,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(50)) }],
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        0,
        10,
        PendingWriteOp::StepWeightRecords,
        &[WeightRecordUpdate { id: 2, weight: constant_weight(pct(50)) }],
    )
    .unwrap();
    assert!(cancel(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some());

    let result = apply_due_writes(&mut streams, 10);
    assert_eq!(1, result.dropped.len());
    assert_eq!(pct(45), streams.streams[1].weight.v_start);
    assert_eq!(8, steps);

    queue_weight_records(
        &mut streams,
        10,
        0,
        PendingWriteOp::SetWeightRecords,
        &[
            WeightRecordUpdate { id: 1, weight: constant_weight(pct(50)) },
            WeightRecordUpdate { id: 2, weight: constant_weight(pct(50)) },
        ],
    )
    .unwrap();
    let result = apply_due_writes(&mut streams, 10);
    assert!(result.dropped.is_empty());
    assert_eq!(pct(50), streams.streams[0].weight.v_start);
    assert_eq!(pct(50), streams.streams[1].weight.v_start);
    assert_eq!(8, steps);
}

#[test]
fn preserves_queue_position_for_equal_epoch_calls() {
    let mut streams = base_state();
    queue_set_distribution(&mut streams, 0, 10, 2, Address::new_id(999)).unwrap();
    queue_remove_stream(&mut streams, 0, 10, 2).unwrap();

    let result = apply_due_writes(&mut streams, 10);
    assert!(result.dropped.is_empty());
    assert_eq!(vec![1], streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>());
}

#[test]
fn removal_takes_the_stream_out_of_the_next_award() {
    let mut streams = base_state();
    let reward = TokenAmount::from_atto(100);
    let before = allocate(&streams.streams, 0, &reward);
    assert_eq!(TokenAmount::from_atto(20), payout(&before, 2, 101));

    queue_remove_stream(&mut streams, 0, 1, 2).unwrap();
    let result = apply_due_writes(&mut streams, 1);
    assert_eq!(
        vec![PendingWriteOp::RemoveStream],
        result.applied.iter().map(|write| write.op).collect::<Vec<_>>()
    );
    assert_eq!(vec![1], streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>());

    let after = allocate(&streams.streams, 0, &reward);
    assert!(after.payouts.is_empty());
    assert_eq!(TokenAmount::from_atto(60), after.miner);
    assert_eq!(TokenAmount::from_atto(40), after.burn);
}

#[test]
fn writer_replacement_repoints_the_stream_without_changing_its_payouts() {
    let mut streams = base_state();
    let reward = TokenAmount::from_atto(100);

    queue_set_distribution(&mut streams, 0, 1, 2, Address::new_id(999)).unwrap();
    let result = apply_due_writes(&mut streams, 1);
    assert!(result.dropped.is_empty());

    let distribution = streams.streams[1].distribution.as_ref().unwrap();
    assert_eq!(Address::new_id(999), distribution.writer);
    assert_eq!(TokenAmount::from_atto(20), payout(&allocate(&streams.streams, 0, &reward), 2, 101));
}

#[test]
fn projects_due_writes_without_mutating_stored_state() {
    let mut streams = base_state();
    queue_weight_records(
        &mut streams,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(70)) }],
    )
    .unwrap();

    let mut projected = streams.clone();
    apply_due_writes(&mut projected, 10);
    assert_eq!(pct(70), projected.streams[0].weight.v_start);
    assert!(projected.pending_writes.is_empty());

    assert_eq!(pct(60), streams.streams[0].weight.v_start);
    assert_eq!(1, streams.pending_writes.len());
    assert_eq!(10, next_epoch(&streams));
}

#[test]
fn randomized_conservation_covers_the_full_operation_mix_and_drops() {
    let mut streams = base_state();
    let mut supply = SupplyTracker::default();
    let mut random = 0x05ee_df02_u64;
    let mut epoch = 0;
    let mut next_id = 4;
    let mut covered = [false; 8];
    let mut paying_awards = 0;

    queue_register_stream(
        &mut streams,
        epoch,
        2,
        stream(3, 0, Some(explicit(203, shares(&[(103, DENOM)])))),
        epoch + 2,
    )
    .unwrap();
    queue_weight_records(
        &mut streams,
        epoch,
        2,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 3, weight: constant_weight(pct(10)) }],
    )
    .unwrap();
    queue_set_distribution(&mut streams, epoch, 2, 3, Address::new_id(303)).unwrap();
    assert!(cancel(&mut streams, Some(3), PendingWriteOp::RegisterStream).unwrap().is_some());
    epoch += 2;
    let result = apply_due_writes(&mut streams, epoch);
    assert_eq!(2, result.dropped.len());
    let mut dropped = result.dropped.len();

    for _ in 0..512 {
        epoch += 1;
        let result = apply_due_writes(&mut streams, epoch);
        dropped += result.dropped.len();

        let previous =
            (supply.total_minted.clone(), supply.total_burn.clone(), supply.total_explicit.clone());
        match random_u64(&mut random) % covered.len() as u64 {
            0 => {
                let reward = TokenAmount::from_atto(random_u64(&mut random) % 1_000 + 1);
                if supply.award(&streams, epoch, reward) > 0 {
                    paying_awards += 1;
                }
                covered[0] = true;
            }
            1 => {
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
                    // A rejected call leaves the ledger unspecified, so keep the copy only when
                    // it was accepted.
                    let mut next_streams = streams.clone();
                    if set_shares(&mut next_streams, id, new_shares).is_ok() {
                        streams = next_streams;
                        covered[1] = true;
                    }
                }
            }
            2 => {
                let id = next_id;
                // A weight the envelope has room for, so a registered stream is paid like the
                // ones the base state starts with.
                let new_stream = stream(
                    id,
                    pct(2),
                    Some(explicit(20_000 + id, shares(&[(30_000 + id, DENOM)]))),
                );
                if queue_register_stream(&mut streams, epoch, 2, new_stream, epoch + 2).is_ok() {
                    next_id += 1;
                    covered[2] = true;
                }
            }
            3 => {
                let ids: Vec<_> = streams
                    .streams
                    .iter()
                    .filter(|stream| stream.distribution.is_some())
                    .map(|stream| stream.id)
                    .collect();
                if !ids.is_empty() {
                    let id = ids[random_u64(&mut random) as usize % ids.len()];
                    if queue_remove_stream(&mut streams, epoch, 2, id).is_ok() {
                        covered[3] = true;
                    }
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
                    let writer = Address::new_id(40_000 + random_u64(&mut random) % 32);
                    if queue_set_distribution(&mut streams, epoch, 2, id, writer).is_ok() {
                        covered[4] = true;
                    }
                }
            }
            5 | 6 => {
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
                if queue_weight_records(&mut streams, epoch, 2, op, &[update]).is_ok() {
                    covered[if op == PendingWriteOp::SetWeightRecords { 5 } else { 6 }] = true;
                }
            }
            7 => {
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
                        covered[7] = true;
                    }
                }
            }
            _ => unreachable!(),
        }

        assert!(supply.total_minted >= previous.0);
        assert!(supply.total_burn >= previous.1);
        assert!(supply.total_explicit >= previous.2);
        supply.assert_invariants();
        validate_streams_state(&streams, epoch).unwrap();
    }

    let result = apply_due_writes(&mut streams, epoch + 2);
    dropped += result.dropped.len();
    supply.assert_invariants();
    assert!(covered.iter().all(|covered| *covered), "missing operation coverage: {covered:?}");
    assert!(dropped >= 2);
    assert!(
        paying_awards >= 32,
        "only {paying_awards} awards paid a recipient, so the mix left no explicit stream live"
    );
}
