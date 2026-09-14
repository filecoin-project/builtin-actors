//! Mainnet boostratp activation according to FIP-0118, modelled here as it's intended to be
//! implemented.

use fil_actors_runtime::EPOCHS_IN_DAY;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::*;
use crate::streams::invariants::{schedule, structure};

const ACTIVATION: ChainEpoch = 6_400_000; // Not exact, estimate at time of writing
const TIMELOCK: ChainEpoch = 7 * EPOCHS_IN_DAY;
const QUARTER_EPOCHS: ChainEpoch = 262_800; // 91.25 days
const RAMP_EPOCHS: ChainEpoch = 9 * QUARTER_EPOCHS;
const CONSENSUS: (u64, u64, u64) = (95, 50, 95);
const SERVICE: (u64, u64, u64) = (5, 5, 10);
const SRA: u64 = 1_000;
const ORCHESTRATOR: u64 = 1_001;

fn bootstrap_record((v_start, floor, cap): (u64, u64, u64), slope: i64) -> WeightRecord {
    WeightRecord {
        v_start: pct(v_start),
        slope,
        t_start: ACTIVATION,
        floor: pct(floor),
        cap: pct(cap),
    }
}

fn bootstrap_slope() -> i64 {
    let total = pct(CONSENSUS.0) - pct(CONSENSUS.1);
    let epochs = RAMP_EPOCHS as u64;
    let slope = total / epochs + u64::from(!total.is_multiple_of(epochs));
    i64::try_from(slope).unwrap()
}

// FIP-0118's w1 & w2 split slope over the bootstrap quarter.
fn split_bootstrap(consensus_slope: i64, service_slope: i64) -> (StreamsState, Vec<StreamAccrual>) {
    let streams = StreamsState {
        streams: vec![
            Stream {
                id: 1,
                weight: bootstrap_record(CONSENSUS, consensus_slope),
                distribution: None,
            },
            Stream {
                id: 2,
                weight: bootstrap_record(SERVICE, service_slope),
                distribution: Some(explicit(SRA, shares(&[(ORCHESTRATOR, DENOM)]))),
            },
        ],
        ..Default::default()
    };
    (streams, vec![StreamAccrual { id: 2, amount: TokenAmount::zero() }])
}

// pre-FIP-0118 status-quo, consensus stream only at 100%.
fn neutral_bootstrap() -> (StreamsState, Vec<StreamAccrual>) {
    let weight =
        WeightRecord { v_start: DENOM, slope: 0, t_start: ACTIVATION, floor: DENOM, cap: DENOM };
    (
        StreamsState {
            streams: vec![Stream { id: 1, weight, distribution: None }],
            ..Default::default()
        },
        vec![],
    )
}

fn weights_at(streams: &StreamsState, epoch: ChainEpoch) -> (u64, u64) {
    let evaluated = schedule_at(&streams.streams, epoch).unwrap();
    (evaluated[0], *evaluated.get(1).unwrap_or(&0))
}

/// One full award through the engine's award path.
fn award_at(streams: &StreamsState, accruals: &[StreamAccrual], epoch: ChainEpoch) -> Allocation {
    let block_reward = TokenAmount::from_whole(5);
    let balance = TokenAmount::from_whole(1_000_000_000);
    let ledger = Ledger::checked(streams.clone(), accruals.to_vec()).unwrap();
    let (_, award) = plan_award(ledger, epoch, &balance, &TokenAmount::zero(), &block_reward)
        .expect("a valid bootstrap always awards in full");
    assert_eq!(block_reward, award.block_reward);
    let explicit: TokenAmount = award.allocation.portions.iter().map(|(_, amount)| amount).sum();
    assert_eq!(block_reward, &award.allocation.miner + &explicit + &award.allocation.burn);
    assert!(award.applied.applied.is_empty() && award.applied.dropped.is_empty());
    award.allocation
}

fn floor_portion(block_reward: &TokenAmount, weight: u64) -> TokenAmount {
    TokenAmount::from_atto(block_reward.atto() * weight / BigInt::from(DENOM))
}

#[test]
fn mainnet_bootstrap_loads_and_holds_its_schedule_forever() {
    let slope = bootstrap_slope();
    let (streams, accruals) = split_bootstrap(-slope, slope);

    // The load every explicit method runs, then the full checker at activation.
    Ledger::checked(streams.clone(), accruals.clone()).unwrap();
    validate_streams_state(&streams, &accruals, ACTIVATION).unwrap();
    // And from every later vantage: the award applies due writes from their own effective epoch
    // and the schedule check runs from there.
    for from in
        [ACTIVATION + 1, ACTIVATION + TIMELOCK, ACTIVATION + RAMP_EPOCHS, ChainEpoch::MAX - 1]
    {
        schedule(&streams.streams, from).unwrap();
    }
}

#[test]
fn mainnet_bootstrap_sums_to_denom_through_the_service_ramp_then_falls() {
    let slope = bootstrap_slope();
    let (streams, _) = split_bootstrap(-slope, slope);
    let service_cap_epoch = ACTIVATION
        + ChainEpoch::try_from((pct(SERVICE.2) - pct(SERVICE.0)).div_ceil(slope as u64)).unwrap();

    // Symmetric slopes keep the sum at exactly DENOM until the service record meets its cap, so
    // nothing but rounding burns through the bootstrap quarter.
    assert_eq!((pct(95), pct(5)), weights_at(&streams, ACTIVATION));
    let mut epoch = ACTIVATION;
    while epoch < service_cap_epoch {
        let (consensus, service) = weights_at(&streams, epoch);
        assert_eq!(DENOM, consensus + service, "sum drifts at epoch {epoch}");
        epoch += EPOCHS_IN_DAY / 4;
    }
    for epoch in [service_cap_epoch - 1, service_cap_epoch, service_cap_epoch + 1] {
        let (consensus, service) = weights_at(&streams, epoch);
        assert!(consensus + service <= DENOM);
        assert!(service <= pct(SERVICE.2));
    }
    assert_eq!(pct(SERVICE.2), weights_at(&streams, service_cap_epoch).1);
    // The service ramp is one bootstrap quarter, within a day of it.
    assert!((service_cap_epoch - ACTIVATION - QUARTER_EPOCHS).abs() < EPOCHS_IN_DAY);

    // Rounding the slope up lands the consensus record on its floor at the nominal ramp end and
    // from there both records are flat and the residual burns.
    let ramp_end = ACTIVATION + RAMP_EPOCHS;
    assert!(weights_at(&streams, ramp_end - 1).0 > pct(CONSENSUS.1));
    assert_eq!(pct(CONSENSUS.1), weights_at(&streams, ramp_end).0);
    assert_eq!((pct(50), pct(10)), weights_at(&streams, ramp_end + 10 * 365 * EPOCHS_IN_DAY));
}

#[test]
fn mainnet_bootstrap_awards_in_full_at_every_stage() {
    let slope = bootstrap_slope();
    let (mut streams, mut accruals) = split_bootstrap(-slope, slope);
    let block_reward = TokenAmount::from_whole(5);
    let ramp_end = ACTIVATION + RAMP_EPOCHS;

    for epoch in [
        ACTIVATION,
        ACTIVATION + 1,
        ACTIVATION + QUARTER_EPOCHS / 2,
        ACTIVATION + QUARTER_EPOCHS,
        ACTIVATION + QUARTER_EPOCHS + EPOCHS_IN_DAY,
        ramp_end - 1,
        ramp_end,
        ramp_end + 10 * 365 * EPOCHS_IN_DAY,
    ] {
        let (consensus, service) = weights_at(&streams, epoch);
        let allocation = award_at(&streams, &accruals, epoch);
        assert_eq!(floor_portion(&block_reward, consensus), allocation.miner, "miner at {epoch}");
        assert_eq!(vec![(2, floor_portion(&block_reward, service))], allocation.portions);
        let residual = &block_reward - &allocation.miner - &allocation.portions[0].1;
        assert_eq!(residual, allocation.burn, "burn at {epoch}");
        if consensus + service == DENOM {
            assert!(allocation.burn <= TokenAmount::from_atto(1), "rounding only at {epoch}");
        }
        accrue(&mut accruals, &allocation.portions);
    }

    // Lone orchestrator can take the whole accrual.
    let liability = explicit_liabilities(&streams, &accruals);
    assert!(liability > TokenAmount::zero());
    let paid = claim(&mut streams, &accruals, 2, &[Address::new_id(ORCHESTRATOR)]).unwrap();
    assert_eq!(vec![liability], paid);
    Ledger::checked(streams, accruals).unwrap();
}

#[test]
fn neutral_bootstrap_pays_the_miner_everything() {
    let (streams, accruals) = neutral_bootstrap();
    validate_streams_state(&streams, &accruals, ACTIVATION).unwrap();
    for epoch in [ACTIVATION, ACTIVATION + RAMP_EPOCHS] {
        let allocation = award_at(&streams, &accruals, epoch);
        assert_eq!(TokenAmount::from_whole(5), allocation.miner);
        assert!(allocation.portions.is_empty());
        assert!(allocation.burn.is_zero());
    }
}

#[test]
fn checker_rejects_a_bootstrap_off_by_one_unit() {
    let slope = bootstrap_slope();

    // The service record climbs one unit per epoch faster than the consensus record falls, so the
    // sum breaches DENOM at the second epoch and stays over until the service cap.
    let (streams, accruals) = split_bootstrap(-slope, slope + 1);
    let error = validate_streams_state(&streams, &accruals, ACTIVATION).unwrap_err();
    assert!(error.to_string().contains("exceed DENOM"), "{error}");
    structure(&streams).unwrap();
    assert!(schedule_at(&streams.streams, ACTIVATION).is_ok());
    assert!(schedule_at(&streams.streams, ACTIVATION + 1).is_err());

    // The opposite asymmetry is valid: the sum only ever falls below DENOM.
    let (streams, accruals) = split_bootstrap(-slope, slope - 1);
    validate_streams_state(&streams, &accruals, ACTIVATION).unwrap();

    // One unit over at the start is over from the first block.
    let (mut streams, accruals) = split_bootstrap(-slope, slope);
    streams.streams[1].weight.v_start += 1;
    let error = validate_streams_state(&streams, &accruals, ACTIVATION).unwrap_err();
    assert!(error.to_string().contains(&format!("at epoch {ACTIVATION}")), "{error}");
    assert!(schedule_at(&streams.streams, ACTIVATION).is_err());
}

/// From an accidental over-DENOM bootstrap, the award pays gas only, but the state still loads and
/// a `SetWeightRecords` repair is admissible and takes effect after the timelock.
#[test]
fn an_over_denom_bootstrap_is_repairable_through_the_queue() {
    let slope = bootstrap_slope();
    let (mut streams, mut accruals) = split_bootstrap(-slope, slope + 1);
    let balance = TokenAmount::from_whole(1_000_000_000);
    let block_reward = TokenAmount::from_whole(5);

    let broken = ACTIVATION + 1;
    let ledger = Ledger::checked(streams.clone(), accruals.clone()).unwrap();
    assert!(plan_award(ledger, broken, &balance, &TokenAmount::zero(), &block_reward).is_none());

    let repair = [WeightRecordUpdate { id: 2, weight: bootstrap_record(SERVICE, slope) }];
    let queued = queue_weight_records(
        &mut streams,
        &accruals,
        broken,
        TIMELOCK,
        PendingWriteOp::SetWeightRecords,
        &repair,
    )
    .unwrap();
    assert_eq!(broken + TIMELOCK, queued.effective_epoch);

    let ledger = Ledger::checked(streams.clone(), accruals.clone()).unwrap();
    assert!(
        plan_award(ledger, broken + TIMELOCK - 1, &balance, &TokenAmount::zero(), &block_reward)
            .is_none()
    );
    let ledger = Ledger::checked(streams.clone(), accruals.clone()).unwrap();
    let (ledger, award) =
        plan_award(ledger, broken + TIMELOCK, &balance, &TokenAmount::zero(), &block_reward)
            .unwrap();
    assert_eq!(1, award.applied.applied.len());
    assert!(award.allocation.miner > TokenAmount::zero());
    streams = ledger.streams;
    accruals = ledger.accrued;
    validate_streams_state(&streams, &accruals, broken + TIMELOCK).unwrap();
}
