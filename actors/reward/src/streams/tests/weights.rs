use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use num_traits::ToPrimitive;

use super::*;
use crate::streams::weights::{
    compute_weight, validate_weight_record, validate_weight_schedule, weight_breakpoints,
};

fn compute_weight_unbounded(record: &WeightRecord, epoch: ChainEpoch) -> u64 {
    let delta = BigInt::from(epoch) - BigInt::from(record.t_start);
    let value = BigInt::from(record.v_start) + BigInt::from(record.slope) * delta;
    value.min(BigInt::from(record.cap)).max(BigInt::from(record.floor)).to_u64().unwrap()
}

#[test]
fn computes_weights_without_overflow_at_integer_boundaries() {
    let values = [0, 1, DENOM, u64::MAX];
    let slopes = [i64::MIN, i64::MIN + 1, -1, 0, 1, i64::MAX];
    let epochs = [i64::MIN, -1, 0, 1, i64::MAX];
    let bounds = [0, DENOM, u64::MAX];

    for &v_start in &values {
        for &slope in &slopes {
            for &t_start in &epochs {
                for &epoch in &epochs {
                    for &floor in &bounds {
                        for &cap in &bounds {
                            let record = WeightRecord { v_start, slope, t_start, floor, cap };
                            assert_eq!(
                                compute_weight_unbounded(&record, epoch),
                                compute_weight(&record, epoch),
                                "record {record:?} at epoch {epoch}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn computes_signed_clamped_weights() {
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

    let overflowing = WeightRecord {
        v_start: u64::MAX,
        slope: i64::MIN,
        t_start: i64::MAX,
        floor: 0,
        cap: u64::MAX,
    };
    assert_eq!(u64::MAX, compute_weight(&overflowing, i64::MIN));

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
