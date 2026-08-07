use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::clock::EPOCH_UNDEFINED;
use fvm_shared::econ::TokenAmount;
use hex_literal::hex;
use num_traits::Zero;

use super::*;

fn pct(value: u64) -> u64 {
    DENOM / 100 * value
}

fn constant_weight(value: u64) -> WeightRecord {
    WeightRecord { v_start: value, slope: 0, t_start: 0, floor: value, cap: value }
}

fn shares(rows: &[(u64, u64)]) -> Vec<RecipientShare> {
    rows.iter()
        .map(|(id, share)| RecipientShare { recipient: Address::new_id(*id), share: *share })
        .collect()
}

fn explicit(writer: u64, shares: Vec<RecipientShare>) -> ExplicitDistribution {
    ExplicitDistribution {
        writer: Address::new_id(writer),
        shares,
        payable: Vec::new(),
        claimed_period: Vec::new(),
    }
}

fn stream(id: StreamId, weight: u64, distribution: Option<ExplicitDistribution>) -> Stream {
    Stream { id, weight: constant_weight(weight), distribution }
}

fn base_state() -> (StreamsState, Vec<StreamAccrual>) {
    (
        StreamsState {
            streams: vec![
                stream(1, pct(60), None),
                stream(2, pct(20), Some(explicit(200, shares(&[(101, DENOM)])))),
            ],
            tombstones: Vec::new(),
            pending_writes: Vec::new(),
        },
        vec![StreamAccrual { id: 2, amount: TokenAmount::zero() }],
    )
}

fn next_epoch(streams: &StreamsState) -> ChainEpoch {
    streams.pending_writes.first().map_or(EPOCH_UNDEFINED, |write| write.effective_epoch)
}

fn tombstone(id: StreamId, first_recipient: u64, rows: usize) -> Tombstone {
    Tombstone {
        id,
        payable: (0..rows)
            .map(|offset| RecipientAmount {
                recipient: Address::new_id(first_recipient + offset as u64),
                amount: TokenAmount::from_atto(1),
            })
            .collect(),
    }
}

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
    queue_remove_stream(&mut at_boundary, 0, 1, 2).unwrap();

    let mut over = base;
    over.tombstones = vec![tombstone(3, 1_000, 193)];
    let error = queue_remove_stream(&mut over, 0, 1, 2).unwrap_err();
    assert!(error.to_string().contains("tombstone row reservation 257"), "{error}");

    let paid = claim(&mut over, &accruals, 3, &[Address::new_id(1_000)]).unwrap();
    assert_eq!(vec![TokenAmount::from_atto(1)], paid);
    queue_remove_stream(&mut over, 0, 1, 2).unwrap();
}

#[test]
fn rejects_persisted_tombstone_rows_above_the_bound() {
    let (mut streams, accruals) = base_state();
    streams.tombstones = vec![tombstone(3, 1_000, MAX_TOMBSTONE_ROWS + 1)];
    let error = validate_streams_state(&streams, &accruals, 0).unwrap_err();
    assert!(error.to_string().contains("tombstone row reservation 257"), "{error}");
}

#[test]
fn pending_removal_bound_survives_share_folds_during_the_hold() {
    let (mut streams, mut accruals) = base_state();
    streams.tombstones = vec![tombstone(3, 1_000, 192)];
    accruals[0].amount = TokenAmount::from_atto(1);
    queue_remove_stream(&mut streams, 0, 1, 2).unwrap();

    let share = DENOM / MAX_RECIPIENTS as u64;
    let replacement: Vec<_> = (0..MAX_RECIPIENTS)
        .map(|offset| RecipientShare { recipient: Address::new_id(2_000 + offset as u64), share })
        .collect();
    let before = streams.clone();
    let before_accruals = accruals.clone();
    let error = set_shares(&mut streams, &mut accruals, 2, replacement.clone()).unwrap_err();
    assert!(error.to_string().contains("tombstone row reservation 257"), "{error}");
    assert_eq!(before, streams);
    assert_eq!(before_accruals, accruals);

    claim(&mut streams, &accruals, 3, &[Address::new_id(1_000)]).unwrap();
    set_shares(&mut streams, &mut accruals, 2, replacement).unwrap();
    apply_due_writes(&mut streams, &mut accruals, 1).unwrap();
    let rows: usize = streams.tombstones.iter().map(|row| row.payable.len()).sum();
    assert!(rows <= MAX_TOMBSTONE_ROWS);
}

fn amount(rows: &[RecipientAmount], recipient: u64) -> TokenAmount {
    rows.iter()
        .find(|row| row.recipient == Address::new_id(recipient))
        .map_or_else(TokenAmount::zero, |row| row.amount.clone())
}

fn service_liabilities(streams: &StreamsState, accruals: &[StreamAccrual]) -> TokenAmount {
    compute_service_liability(streams, accruals).unwrap()
}

fn assert_service_conserved(
    gross: &TokenAmount,
    paid: &TokenAmount,
    burned: &TokenAmount,
    streams: &StreamsState,
    accruals: &[StreamAccrual],
) {
    let mut accounted = paid.clone();
    accounted += burned;
    accounted += service_liabilities(streams, accruals);
    assert_eq!(*gross, accounted);
}

#[derive(Default)]
struct SupplyTracker {
    total_minted: TokenAmount,
    total_burn: TokenAmount,
    total_service: TokenAmount,
    total_dust: TokenAmount,
    f099_balance: TokenAmount,
    actor_balance: TokenAmount,
}

impl SupplyTracker {
    fn award(
        &mut self,
        streams: &StreamsState,
        accruals: &mut [StreamAccrual],
        epoch: ChainEpoch,
        reward: TokenAmount,
    ) {
        let allocation = allocate_reward(&streams.streams, epoch, &reward).unwrap();
        assert!(allocation.schedule_valid, "valid randomized state entered degradation");
        let service =
            allocation.service.iter().fold(TokenAmount::zero(), |total, row| total + &row.amount);
        accrue_service(accruals, &allocation.service).unwrap();
        self.total_minted += &reward;
        self.total_burn += &allocation.burn;
        self.total_service += &service;
        self.f099_balance += allocation.burn;
        self.actor_balance += service;
    }

    fn burn_dust(&mut self, dust: TokenAmount) {
        self.actor_balance -= &dust;
        self.total_dust += &dust;
        self.f099_balance += dust;
    }

    fn pay_claim(&mut self, result: &[TokenAmount]) {
        let paid = result.iter().fold(TokenAmount::zero(), |total, amount| total + amount);
        self.actor_balance -= paid;
    }

    fn assert_invariants(&self, streams: &StreamsState, accruals: &[StreamAccrual]) {
        let liabilities = service_liabilities(streams, accruals);
        assert_eq!(self.actor_balance, liabilities);
        assert!(
            liabilities <= self.total_service,
            "conservative service reserve is below exact liability"
        );
        let miner = &self.total_minted - &self.total_burn - &self.total_service;
        assert!(miner >= TokenAmount::zero());
        assert_eq!(self.f099_balance, &self.total_burn + &self.total_dust);
    }
}

fn random_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    *state
}

#[test]
fn computes_signed_clamped_weights_without_overflow() {
    let rising = WeightRecord { v_start: 10, slope: 2, t_start: 5, floor: 4, cap: 20 };
    assert_eq!(4, compute_weight(&rising, 0));
    assert_eq!(10, compute_weight(&rising, 5));
    assert_eq!(20, compute_weight(&rising, 10));

    let falling = WeightRecord { v_start: 10, slope: -2, t_start: 5, floor: 4, cap: 20 };
    assert_eq!(20, compute_weight(&falling, 0));
    assert_eq!(10, compute_weight(&falling, 5));
    assert_eq!(4, compute_weight(&falling, 10));

    let flat = WeightRecord { v_start: 20, slope: 0, t_start: 0, floor: 4, cap: 20 };
    assert_eq!(20, compute_weight(&flat, i64::MIN));
    assert_eq!(20, compute_weight(&flat, i64::MAX));

    let extreme =
        WeightRecord { v_start: DENOM, slope: i64::MIN, t_start: i64::MAX, floor: 0, cap: DENOM };
    assert_eq!(DENOM, compute_weight(&extreme, i64::MIN));

    let reversed = WeightRecord { v_start: 10, slope: 0, t_start: 0, floor: 20, cap: 4 };
    assert_eq!(20, compute_weight(&reversed, 0));
}

#[test]
fn evaluates_a_delayed_canonical_ramp_flat_then_rising() {
    let delayed = WeightRecord { v_start: 10, slope: 2, t_start: 5, floor: 10, cap: 20 };
    validate_weight_record(&delayed).unwrap();
    assert_eq!(10, compute_weight(&delayed, 0));
    assert_eq!(10, compute_weight(&delayed, 4));
    assert_eq!(10, compute_weight(&delayed, 5));
    assert_eq!(12, compute_weight(&delayed, 6));
    assert_eq!(20, compute_weight(&delayed, 10));
}

#[test]
fn finds_clamp_transition_edges_at_large_epochs() {
    let record = WeightRecord { v_start: 10, slope: 3, t_start: 100, floor: 4, cap: 20 };
    let epochs = weight_breakpoints(&record, 0);
    for epoch in [0, 97, 98, 99, 100, 102, 103, 104, 105] {
        assert!(epochs.contains(&epoch), "missing breakpoint {epoch}");
    }

    let record =
        WeightRecord { v_start: 10, slope: 1, t_start: i64::MAX - 1, floor: 0, cap: DENOM };
    let epochs = weight_breakpoints(&record, i64::MAX - 2);
    assert!(epochs.contains(&i64::MAX));
}

#[test]
fn validates_weight_records_and_piecewise_schedule() {
    assert!(
        validate_weight_record(&WeightRecord {
            v_start: 0,
            slope: 0,
            t_start: 0,
            floor: 2,
            cap: 1,
        })
        .is_err()
    );
    assert!(validate_weight_record(&constant_weight(DENOM + 1)).is_err());

    let valid = vec![
        stream(1, pct(60), None),
        stream(2, pct(40), Some(explicit(200, shares(&[(101, DENOM)])))),
    ];
    validate_weight_schedule(&valid, 0).unwrap();

    let invalid = vec![
        stream(1, pct(60), None),
        stream(2, pct(41), Some(explicit(200, shares(&[(101, DENOM)])))),
    ];
    let error = validate_weight_schedule(&invalid, 0).unwrap_err();
    assert!(error.to_string().contains("exceed DENOM"));

    let crossing = vec![
        Stream {
            id: 1,
            weight: WeightRecord {
                v_start: pct(50),
                slope: 1,
                t_start: 0,
                floor: pct(50),
                cap: pct(70),
            },
            distribution: None,
        },
        stream(2, pct(40), Some(explicit(200, shares(&[(101, DENOM)])))),
    ];
    assert!(validate_weight_schedule(&crossing, 0).is_err());

    let near_end = i64::MAX - 100;
    let endpoint_overlap = vec![
        Stream {
            id: 1,
            weight: WeightRecord {
                v_start: pct(49),
                slope: pct(1) as i64,
                t_start: near_end,
                floor: 0,
                cap: DENOM,
            },
            distribution: None,
        },
        Stream {
            id: 2,
            weight: WeightRecord {
                v_start: pct(49),
                slope: pct(1) as i64,
                t_start: near_end,
                floor: 0,
                cap: DENOM,
            },
            distribution: Some(explicit(200, shares(&[(101, DENOM)]))),
        },
    ];
    assert!(validate_weight_schedule(&endpoint_overlap, near_end).is_err());
}

#[test]
fn solstice_migration_bootstrap_records_have_canonical_anchors() {
    let activation = 1_000;
    let slope = (pct(45) / 90) as i64;
    let consensus = WeightRecord {
        v_start: pct(95),
        slope: -slope,
        t_start: activation,
        floor: pct(50),
        cap: pct(95),
    };
    let service =
        WeightRecord { v_start: pct(5), slope, t_start: activation, floor: pct(5), cap: pct(10) };

    validate_weight_record(&consensus).unwrap();
    validate_weight_record(&service).unwrap();
    validate_weight_schedule(
        &[
            Stream { id: 1, weight: consensus, distribution: None },
            Stream {
                id: 2,
                weight: service,
                distribution: Some(explicit(200, shares(&[(101, DENOM)]))),
            },
        ],
        activation,
    )
    .unwrap();
}

#[test]
fn rejects_out_of_band_anchors_on_every_weight_queue_operation() {
    let invalid = [
        ("below floor", WeightRecord { v_start: 10, slope: 1, t_start: 0, floor: 20, cap: 30 }),
        ("exceeds cap", WeightRecord { v_start: 30, slope: -1, t_start: 0, floor: 10, cap: 20 }),
    ];

    for op in [PendingWriteOp::SetWeightRecords, PendingWriteOp::StepWeightRecords] {
        for (message, weight) in &invalid {
            let (mut streams, _) = base_state();
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
        let (mut streams, _) = base_state();
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
fn allocates_reward_in_stream_order_and_conserves_attos() {
    let streams = vec![
        stream(1, pct(50), None),
        stream(2, pct(25), Some(explicit(200, shares(&[(101, DENOM)])))),
        stream(3, pct(10), Some(explicit(201, shares(&[(102, DENOM)])))),
    ];
    let reward = TokenAmount::from_atto(7);
    let allocation = allocate_reward(&streams, 0, &reward).unwrap();

    assert_eq!(TokenAmount::from_atto(3), allocation.miner);
    assert_eq!(vec![2, 3], allocation.service.iter().map(|row| row.id).collect::<Vec<_>>());
    assert_eq!(TokenAmount::from_atto(1), allocation.service[0].amount);
    assert_eq!(TokenAmount::zero(), allocation.service[1].amount);
    assert_eq!(TokenAmount::from_atto(3), allocation.burn);
    assert!(allocation.schedule_valid);
    assert_eq!(
        reward,
        &allocation.miner
            + &allocation.service.iter().fold(TokenAmount::zero(), |sum, row| sum + &row.amount)
            + &allocation.burn
    );

    let mut accruals = vec![
        StreamAccrual { id: 2, amount: TokenAmount::from_atto(4) },
        StreamAccrual { id: 3, amount: TokenAmount::from_atto(5) },
    ];
    accrue_service(&mut accruals, &allocation.service).unwrap();
    assert_eq!(TokenAmount::from_atto(5), accruals[0].amount);
    assert_eq!(TokenAmount::from_atto(5), accruals[1].amount);

    let before = accruals.clone();
    assert!(
        accrue_service(
            &mut accruals,
            &[
                StreamAccrual { id: 2, amount: TokenAmount::from_atto(1) },
                StreamAccrual { id: 99, amount: TokenAmount::from_atto(1) },
            ],
        )
        .is_err()
    );
    assert_eq!(before, accruals);
}

#[test]
fn invalid_weight_envelope_allocates_no_reward_portion() {
    let streams = vec![
        stream(1, pct(60), None),
        stream(2, pct(50), Some(explicit(200, shares(&[(101, DENOM)])))),
    ];
    let reward = TokenAmount::from_atto(7);

    let allocation = allocate_reward(&streams, 0, &reward).unwrap();

    assert!(!allocation.schedule_valid);
    assert_eq!(TokenAmount::zero(), allocation.miner);
    assert!(allocation.service.is_empty());
    assert_eq!(TokenAmount::zero(), allocation.burn);
}

#[test]
fn malformed_implicit_weight_allocates_no_reward_portion() {
    let mut malformed = stream(1, 0, None);
    malformed.weight.cap = DENOM + 1;
    let streams = vec![malformed, stream(2, pct(20), Some(explicit(200, shares(&[(101, DENOM)]))))];
    let reward = TokenAmount::from_atto(7);

    let allocation = allocate_reward(&streams, 0, &reward).unwrap();

    assert!(!allocation.schedule_valid);
    assert_eq!(TokenAmount::zero(), allocation.miner);
    assert!(allocation.service.is_empty());
    assert_eq!(TokenAmount::zero(), allocation.burn);
}

#[test]
fn malformed_explicit_weight_allocates_no_reward_portion() {
    let mut malformed = stream(2, 0, Some(explicit(200, shares(&[(101, DENOM)]))));
    malformed.weight.cap = DENOM + 1;
    let reward = TokenAmount::from_atto(7);

    let allocation = allocate_reward(&[malformed], 0, &reward).unwrap();

    assert!(!allocation.schedule_valid);
    assert_eq!(TokenAmount::zero(), allocation.miner);
    assert!(allocation.service.is_empty());
    assert_eq!(TokenAmount::zero(), allocation.burn);
}

#[test]
fn rejects_invalid_share_maps() {
    assert!(validate_shares(&shares(&[(101, DENOM - 1)])).is_err());
    assert!(validate_shares(&shares(&[(101, DENOM / 2), (101, DENOM / 2)])).is_err());
    assert!(validate_shares(&shares(&[(101, DENOM), (102, 0)])).is_err());

    let too_many: Vec<_> = (0..=MAX_RECIPIENTS)
        .map(|idx| RecipientShare {
            recipient: Address::new_id(100 + idx as u64),
            share: if idx == 0 { DENOM } else { 0 },
        })
        .collect();
    assert!(validate_shares(&too_many).is_err());
}

#[test]
fn folds_period_under_outgoing_shares_and_burns_only_dust() {
    let third = DENOM / 3;
    let old_shares = shares(&[(101, third), (102, third), (103, DENOM - 2 * third)]);
    let mut distribution = explicit(200, old_shares);
    distribution.payable.push(RecipientAmount {
        recipient: Address::new_id(104),
        amount: TokenAmount::from_atto(5),
    });
    distribution.claimed_period.push(RecipientAmount {
        recipient: Address::new_id(101),
        amount: TokenAmount::from_atto(1),
    });
    let mut streams = StreamsState {
        streams: vec![stream(2, pct(20), Some(distribution))],
        ..Default::default()
    };
    let mut accruals = vec![StreamAccrual { id: 2, amount: TokenAmount::from_atto(10) }];

    let burn = set_shares(&mut streams, &mut accruals, 2, shares(&[(105, DENOM)])).unwrap();
    let distribution = streams.streams[0].distribution.as_ref().unwrap();
    assert_eq!(TokenAmount::from_atto(1), burn);
    assert_eq!(TokenAmount::zero(), accruals[0].amount);
    assert!(distribution.claimed_period.is_empty());
    assert_eq!(shares(&[(105, DENOM)]), distribution.shares);
    assert_eq!(TokenAmount::from_atto(2), amount(&distribution.payable, 101));
    assert_eq!(TokenAmount::from_atto(3), amount(&distribution.payable, 102));
    assert_eq!(TokenAmount::from_atto(3), amount(&distribution.payable, 103));
    assert_eq!(TokenAmount::from_atto(5), amount(&distribution.payable, 104));
}

#[test]
fn fold_dust_preserves_the_supply_decomposition_without_moving_counters() {
    let (mut streams, mut accruals) = base_state();
    let third = DENOM / 3;
    let split = shares(&[(101, third), (102, third), (103, DENOM - 2 * third)]);
    streams.streams[1].distribution.as_mut().unwrap().shares = split.clone();
    let mut supply = SupplyTracker::default();

    for _ in 0..2 {
        supply.award(&streams, &mut accruals, 0, TokenAmount::from_atto(51));
        let before_burn = supply.total_burn.clone();
        let before_service = supply.total_service.clone();
        let dust = set_shares(&mut streams, &mut accruals, 2, split.clone()).unwrap();
        assert_eq!(TokenAmount::from_atto(1), dust);
        supply.burn_dust(dust);
        assert_eq!(before_burn, supply.total_burn);
        assert_eq!(before_service, supply.total_service);
        supply.assert_invariants(&streams, &accruals);
    }

    assert_eq!(TokenAmount::from_atto(22), supply.total_burn);
    assert_eq!(TokenAmount::from_atto(20), supply.total_service);
    assert_eq!(TokenAmount::from_atto(2), supply.total_dust);
    supply.assert_invariants(&streams, &accruals);
}

#[test]
fn claims_live_and_payable_amounts_once_in_request_order() {
    let mut distribution = explicit(200, shares(&[(101, DENOM / 2), (102, DENOM - DENOM / 2)]));
    distribution.payable = vec![
        RecipientAmount { recipient: Address::new_id(101), amount: TokenAmount::from_atto(3) },
        RecipientAmount { recipient: Address::new_id(102), amount: TokenAmount::from_atto(4) },
    ];
    distribution.claimed_period = vec![RecipientAmount {
        recipient: Address::new_id(101),
        amount: TokenAmount::from_atto(2),
    }];
    let mut streams = StreamsState {
        streams: vec![stream(2, pct(20), Some(distribution))],
        ..Default::default()
    };
    let accruals = vec![StreamAccrual { id: 2, amount: TokenAmount::from_atto(11) }];
    let wallets =
        [Address::new_id(101), Address::new_id(101), Address::new_id(102), Address::new_id(999)];

    let result = claim(&mut streams, &accruals, 2, &wallets).unwrap();
    assert_eq!(
        vec![
            TokenAmount::from_atto(6),
            TokenAmount::zero(),
            TokenAmount::from_atto(9),
            TokenAmount::zero(),
        ],
        result
    );

    let distribution = streams.streams[0].distribution.as_ref().unwrap();
    assert!(distribution.payable.is_empty());
    assert_eq!(TokenAmount::from_atto(5), amount(&distribution.claimed_period, 101));
    assert_eq!(TokenAmount::from_atto(5), amount(&distribution.claimed_period, 102));
    let before = streams.clone();
    let zero = claim(&mut streams, &accruals, 2, &[Address::new_id(999)]).unwrap();
    assert_eq!(vec![TokenAmount::zero()], zero);
    assert_eq!(before, streams);
}

#[test]
fn claims_tombstones_and_deletes_them_when_drained() {
    let mut streams = StreamsState {
        tombstones: vec![Tombstone {
            id: 3,
            payable: vec![
                RecipientAmount {
                    recipient: Address::new_id(101),
                    amount: TokenAmount::from_atto(7),
                },
                RecipientAmount {
                    recipient: Address::new_id(102),
                    amount: TokenAmount::from_atto(8),
                },
            ],
        }],
        ..Default::default()
    };

    let result = claim(
        &mut streams,
        &[],
        3,
        &[Address::new_id(101), Address::new_id(101), Address::new_id(999)],
    )
    .unwrap();
    assert_eq!(vec![TokenAmount::from_atto(7), TokenAmount::zero(), TokenAmount::zero()], result);
    assert_eq!(1, streams.tombstones.len());

    let result = claim(&mut streams, &[], 3, &[Address::new_id(102)]).unwrap();
    assert_eq!(vec![TokenAmount::from_atto(8)], result);

    let before = streams.clone();
    let result =
        claim(&mut streams, &[], 3, &[Address::new_id(102), Address::new_id(999)]).unwrap();
    assert_eq!(vec![TokenAmount::zero(), TokenAmount::zero()], result);
    assert_eq!(before, streams);
}

#[test]
fn rejects_a_new_call_that_strands_an_existing_call() {
    let (mut streams, _) = base_state();

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
    let (mut streams, mut accruals) = base_state();
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

    let result = apply_due_writes(&mut streams, &mut accruals, 2).unwrap();
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
    let (mut streams, _) = base_state();

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
fn queues_batches_cancels_slots_and_tracks_queue_head() {
    let (mut streams, mut accruals) = base_state();
    let updates = [
        WeightRecordUpdate { id: 1, weight: constant_weight(pct(65)) },
        WeightRecordUpdate { id: 2, weight: constant_weight(pct(35)) },
    ];

    assert_eq!(
        17,
        queue_weight_records(&mut streams, 10, 7, PendingWriteOp::SetWeightRecords, &updates,)
            .unwrap()
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

    assert!(cancel_pending(&mut streams, Some(999), PendingWriteOp::SetWeightRecords).is_err());
    assert_eq!(17, next_epoch(&streams));
    assert!(
        cancel_pending(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some()
    );
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
    assert!(
        cancel_pending(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_none()
    );
    assert!(cancel_pending(&mut streams, None, PendingWriteOp::RemoveStream).is_err());

    queue_weight_records(&mut streams, 20, 7, PendingWriteOp::SetWeightRecords, &updates[1..])
        .unwrap();
    assert_eq!(27, next_epoch(&streams));
    assert!(
        cancel_pending(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some()
    );
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));

    queue_weight_records(&mut streams, 10, 7, PendingWriteOp::StepWeightRecords, &updates[..1])
        .unwrap();
    assert!(cancel_pending(&mut streams, None, PendingWriteOp::StepWeightRecords).is_err());
    let result = apply_due_writes_and_cancel(
        &mut streams,
        &mut accruals,
        17,
        None,
        PendingWriteOp::SetWeightRecords,
    )
    .unwrap();
    assert_eq!(TokenAmount::zero(), result.apply_result.burn);
    assert_eq!(
        vec![PendingWriteOp::StepWeightRecords],
        result.apply_result.applied.iter().map(|write| write.op).collect::<Vec<_>>()
    );
    assert!(result.apply_result.dropped.is_empty());
    assert!(result.removed.is_none());
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
    );
    assert_eq!(17, next_epoch(&streams));
    apply_due_writes(&mut streams, &mut accruals, 17).unwrap();
    assert_eq!(pct(30), streams.streams[1].weight.v_start);
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
}

#[test]
fn apply_and_cancel_surfaces_calls_dropped_before_cancellation() {
    let (mut streams, mut accruals) = base_state();
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
    assert!(
        cancel_pending(&mut streams, Some(3), PendingWriteOp::RegisterStream).unwrap().is_some()
    );

    let result = apply_due_writes_and_cancel(
        &mut streams,
        &mut accruals,
        10,
        Some(99),
        PendingWriteOp::RemoveStream,
    )
    .unwrap();
    assert_eq!(1, result.apply_result.dropped.len());
    assert_eq!(PendingWriteOp::SetWeightRecords, result.apply_result.dropped[0].op);
    assert!(result.removed.is_none());
    assert!(streams.pending_writes.is_empty());
}

#[test]
fn enforces_registration_bounds_and_id_availability() {
    let (mut streams, _) = base_state();
    let new_stream =
        stream(3, 0, Some(explicit(201, shares(&[(103, DENOM / 2), (102, DENOM - DENOM / 2)]))));

    assert!(queue_register_stream(&mut streams, 10, 7, new_stream.clone(), 16).is_err());
    assert_eq!(17, queue_register_stream(&mut streams, 10, 7, new_stream, 17).unwrap());
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
    let mut seven_accruals = (2..MAX_STREAMS as u64)
        .map(|id| StreamAccrual { id, amount: TokenAmount::zero() })
        .collect();
    apply_due_writes(&mut seven, &mut seven_accruals, 1).unwrap();
    assert_eq!(MAX_STREAMS, seven.streams.len());

    let mut tombstoned = StreamsState {
        tombstones: vec![Tombstone {
            id: 4,
            payable: vec![RecipientAmount {
                recipient: Address::new_id(101),
                amount: TokenAmount::from_atto(1),
            }],
        }],
        ..Default::default()
    };
    assert!(
        queue_register_stream(
            &mut tombstoned,
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
            0,
            1,
            stream(4, 0, Some(explicit(204, shares(&[(104, DENOM)])))),
            1,
        )
        .unwrap()
    );
}

#[test]
fn rejects_invalid_timelocks_without_mutation() {
    let (mut streams, _) = base_state();
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

    assert!(validate_pending_queue(&writes, None).is_err());
}

#[test]
fn rejects_malformed_pending_order_without_mutation() {
    let (mut streams, mut accruals) = base_state();
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
    let before_streams = streams.clone();
    let before_accruals = accruals.clone();

    assert!(apply_due_writes(&mut streams, &mut accruals, 2).is_err());
    assert_eq!(before_streams, streams);
    assert_eq!(before_accruals, accruals);
    assert_eq!(2, next_epoch(&streams));
}

#[test]
fn rejects_malformed_pending_payload_without_mutation() {
    let (mut streams, mut accruals) = base_state();
    streams.pending_writes.push(PendingWrite {
        id: None,
        op: PendingWriteOp::SetWeightRecords,
        payload: fvm_ipld_encoding::RawBytes::new(vec![0xff]),
        effective_epoch: 1,
    });
    let before_streams = streams.clone();
    let before_accruals = accruals.clone();

    assert!(apply_due_writes(&mut streams, &mut accruals, 1).is_err());
    assert_eq!(before_streams, streams);
    assert_eq!(before_accruals, accruals);
    assert_eq!(1, next_epoch(&streams));
}

#[test]
fn rejects_missing_accrual_state_without_dropping_the_call() {
    let (mut streams, mut accruals) = base_state();
    queue_remove_stream(&mut streams, 0, 1, 2).unwrap();
    accruals.clear();
    let before_streams = streams.clone();

    assert!(apply_due_writes(&mut streams, &mut accruals, 1).is_err());
    assert_eq!(before_streams, streams);
    assert!(accruals.is_empty());
    assert_eq!(1, next_epoch(&streams));
}

#[test]
fn applies_same_epoch_lifecycle_writes_in_operation_order() {
    let (mut streams, mut accruals) = base_state();
    let new_stream = stream(3, 0, Some(explicit(203, shares(&[(103, DENOM)]))));
    queue_register_stream(&mut streams, 0, 10, new_stream, 10).unwrap();
    queue_remove_stream(&mut streams, 0, 10, 3).unwrap();

    apply_due_writes(&mut streams, &mut accruals, 9).unwrap();
    assert_eq!(2, streams.streams.len());
    assert_eq!(10, next_epoch(&streams));

    apply_due_writes(&mut streams, &mut accruals, 10).unwrap();
    assert_eq!(vec![1, 2], streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>());
    assert_eq!(vec![2], accruals.iter().map(|row| row.id).collect::<Vec<_>>());
    assert_eq!(EPOCH_UNDEFINED, next_epoch(&streams));
}

#[test]
fn drops_a_weight_batch_stranded_by_cancelled_registration() {
    let (mut streams, mut accruals) = base_state();
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
    assert!(
        cancel_pending(&mut streams, Some(3), PendingWriteOp::RegisterStream).unwrap().is_some()
    );

    let result = apply_due_writes(&mut streams, &mut accruals, 10).unwrap();
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

    let allocation = allocate_reward(&streams.streams, 10, &TokenAmount::from_atto(100)).unwrap();
    assert_eq!(TokenAmount::from_atto(60), allocation.miner);
    assert_eq!(TokenAmount::from_atto(20), allocation.service[0].amount);
    assert_eq!(TokenAmount::from_atto(20), allocation.burn);
}

#[test]
fn admits_a_timelocked_weight_repair_and_drops_non_repairing_due_writes() {
    let (mut streams, mut accruals) = base_state();
    queue_set_distribution(&mut streams, 0, 2, 2, Address::new_id(999)).unwrap();

    streams.streams[0].weight = constant_weight(pct(90));
    assert!(validate_streams_state(&streams, &accruals, 0).is_err());
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
    assert_eq!(ApplyResult::default(), apply_due_writes(&mut streams, &mut accruals, 1).unwrap());
    assert_eq!(before, streams);
    assert_eq!(2, next_epoch(&streams));

    let result = apply_due_writes(&mut streams, &mut accruals, 2).unwrap();
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
        0,
        2,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(60)) }],
    )
    .unwrap();

    let result = apply_due_writes(&mut streams, &mut accruals, 2).unwrap();

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
    assert!(
        cancel_pending(&mut streams, Some(3), PendingWriteOp::RegisterStream).unwrap().is_some()
    );

    let result = apply_due_writes(&mut streams, &mut accruals, 10).unwrap();
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
    assert!(
        cancel_pending(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some()
    );
    let stranded = project_due_writes(&streams, &accruals, 20).unwrap();
    assert_eq!(1, stranded.apply_result.dropped.len());

    queue_weight_records(
        &mut streams,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(50)) }],
    )
    .unwrap();
    let result = apply_due_writes(&mut streams, &mut accruals, 20).unwrap();
    assert!(result.dropped.is_empty());
    assert_eq!(pct(50), streams.streams[0].weight.v_start);
    assert_eq!(pct(50), streams.streams[1].weight.v_start);
}

#[test]
fn drops_a_stranded_gate_batch_and_accepts_the_next_absolute_level() {
    let (mut streams, mut accruals) = base_state();
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
    assert!(
        cancel_pending(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some()
    );

    let result = apply_due_writes(&mut streams, &mut accruals, 10).unwrap();
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
    let result = apply_due_writes(&mut streams, &mut accruals, 10).unwrap();
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
    assert!(
        cancel_pending(&mut streams, None, PendingWriteOp::SetWeightRecords).unwrap().is_some()
    );

    let result = apply_due_writes(&mut streams, &mut accruals, 10).unwrap();
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
    let result = apply_due_writes(&mut streams, &mut accruals, 10).unwrap();
    assert!(result.dropped.is_empty());
    assert_eq!(pct(50), streams.streams[0].weight.v_start);
    assert_eq!(pct(50), streams.streams[1].weight.v_start);
    assert_eq!(8, steps);
}

#[test]
fn preserves_queue_position_for_equal_epoch_calls() {
    let (mut streams, mut accruals) = base_state();
    queue_set_distribution(&mut streams, 0, 10, 2, Address::new_id(999)).unwrap();
    queue_remove_stream(&mut streams, 0, 10, 2).unwrap();

    let result = apply_due_writes(&mut streams, &mut accruals, 10).unwrap();
    assert!(result.dropped.is_empty());
    assert_eq!(vec![1], streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>());
}

#[test]
fn removal_settles_into_a_claimable_tombstone() {
    let (mut streams, mut accruals) = base_state();
    let distribution = streams.streams[1].distribution.as_mut().unwrap();
    distribution.shares = shares(&[(101, DENOM / 2), (102, DENOM - DENOM / 2)]);
    distribution.payable.push(RecipientAmount {
        recipient: Address::new_id(102),
        amount: TokenAmount::from_atto(3),
    });
    distribution.claimed_period.push(RecipientAmount {
        recipient: Address::new_id(101),
        amount: TokenAmount::from_atto(2),
    });
    accruals[0].amount = TokenAmount::from_atto(11);

    queue_remove_stream(&mut streams, 0, 1, 2).unwrap();
    let result = apply_due_writes(&mut streams, &mut accruals, 1).unwrap();
    assert_eq!(TokenAmount::from_atto(1), result.burn);
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

    queue_set_distribution(&mut streams, 0, 1, 2, Address::new_id(999)).unwrap();
    let result = apply_due_writes(&mut streams, &mut accruals, 1).unwrap();
    let distribution = streams.streams[1].distribution.as_ref().unwrap();
    assert_eq!(TokenAmount::from_atto(1), result.burn);
    assert_eq!(Address::new_id(999), distribution.writer);
    assert_eq!(TokenAmount::from_atto(2), amount(&distribution.payable, 101));
    assert_eq!(TokenAmount::from_atto(2), amount(&distribution.payable, 102));
    assert_eq!(TokenAmount::zero(), accruals[0].amount);
}

#[test]
fn conserves_service_value_across_claims_folds_and_removal() {
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
    assert_service_conserved(&gross, &paid, &burned, &streams, &accruals);

    let result = claim(&mut streams, &accruals, 2, &[Address::new_id(101)]).unwrap();
    paid += &result[0];
    assert_service_conserved(&gross, &paid, &burned, &streams, &accruals);

    burned += set_shares(&mut streams, &mut accruals, 2, shares(&[(104, DENOM)])).unwrap();
    assert_service_conserved(&gross, &paid, &burned, &streams, &accruals);

    accruals[0].amount += TokenAmount::from_atto(5);
    gross += TokenAmount::from_atto(5);
    let result = claim(&mut streams, &accruals, 2, &[Address::new_id(104)]).unwrap();
    paid += &result[0];
    assert_service_conserved(&gross, &paid, &burned, &streams, &accruals);
    queue_remove_stream(&mut streams, 0, 1, 2).unwrap();
    burned += apply_due_writes(&mut streams, &mut accruals, 1).unwrap().burn;
    assert_service_conserved(&gross, &paid, &burned, &streams, &accruals);

    let result =
        claim(&mut streams, &accruals, 2, &[Address::new_id(102), Address::new_id(103)]).unwrap();
    paid += result.iter().fold(TokenAmount::zero(), |total, amount| total + amount);
    assert!(streams.tombstones.is_empty());
    assert_service_conserved(&gross, &paid, &burned, &streams, &accruals);
}

#[test]
fn projects_due_writes_without_mutating_stored_state() {
    let (mut streams, accruals) = base_state();
    queue_weight_records(
        &mut streams,
        0,
        10,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 1, weight: constant_weight(pct(70)) }],
    )
    .unwrap();

    let projected = project_due_writes(&streams, &accruals, 10).unwrap();
    assert_eq!(pct(70), projected.streams.streams[0].weight.v_start);
    assert!(projected.streams.pending_writes.is_empty());
    assert_eq!(TokenAmount::zero(), projected.apply_result.burn);

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
        epoch,
        2,
        stream(3, 0, Some(explicit(203, shares(&[(103, DENOM)])))),
        epoch + 2,
    )
    .unwrap();
    covered[3] = true;
    queue_weight_records(
        &mut streams,
        epoch,
        2,
        PendingWriteOp::SetWeightRecords,
        &[WeightRecordUpdate { id: 3, weight: constant_weight(pct(10)) }],
    )
    .unwrap();
    covered[6] = true;
    queue_set_distribution(&mut streams, epoch, 2, 3, Address::new_id(303)).unwrap();
    covered[5] = true;
    assert!(
        cancel_pending(&mut streams, Some(3), PendingWriteOp::RegisterStream).unwrap().is_some()
    );
    covered[8] = true;
    epoch += 2;
    let result = apply_due_writes(&mut streams, &mut accruals, epoch).unwrap();
    assert_eq!(2, result.dropped.len());
    supply.burn_dust(result.burn);
    let mut dropped = result.dropped.len();

    for _ in 0..512 {
        epoch += 1;
        let result = apply_due_writes(&mut streams, &mut accruals, epoch).unwrap();
        dropped += result.dropped.len();
        supply.burn_dust(result.burn);

        let previous =
            (supply.total_minted.clone(), supply.total_burn.clone(), supply.total_service.clone());
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
                    if let Ok(dust) = set_shares(&mut streams, &mut accruals, id, new_shares) {
                        supply.burn_dust(dust);
                        covered[2] = true;
                    }
                }
            }
            3 => {
                let id = next_id;
                let new_stream =
                    stream(id, 0, Some(explicit(20_000 + id, shares(&[(30_000 + id, DENOM)]))));
                if queue_register_stream(&mut streams, epoch, 2, new_stream, epoch + 2).is_ok() {
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
                    if queue_remove_stream(&mut streams, epoch, 2, id).is_ok() {
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
                    if queue_set_distribution(&mut streams, epoch, 2, id, writer).is_ok() {
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
                if queue_weight_records(&mut streams, epoch, 2, op, &[update]).is_ok() {
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
                    if cancel_pending(&mut streams, id, op).unwrap().is_some() {
                        covered[8] = true;
                    }
                }
            }
            _ => unreachable!(),
        }

        assert!(supply.total_minted >= previous.0);
        assert!(supply.total_burn >= previous.1);
        assert!(supply.total_service >= previous.2);
        supply.assert_invariants(&streams, &accruals);
        validate_streams_state(&streams, &accruals, epoch).unwrap();
    }

    let result = apply_due_writes(&mut streams, &mut accruals, epoch + 2).unwrap();
    dropped += result.dropped.len();
    supply.burn_dust(result.burn);
    supply.assert_invariants(&streams, &accruals);
    assert!(covered.iter().all(|covered| *covered), "missing operation coverage: {covered:?}");
    assert!(dropped >= 2);
}
