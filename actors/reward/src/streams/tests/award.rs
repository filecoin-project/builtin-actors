use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::*;
use crate::streams::invariants::accounting;

#[test]
fn allocates_reward_in_stream_order_and_conserves_attos() {
    let streams = vec![
        stream(1, pct(50), None),
        stream(2, pct(25), Some(explicit(200, shares(&[(101, DENOM)])))),
        stream(3, pct(10), Some(explicit(201, shares(&[(102, DENOM)])))),
    ];
    let reward = TokenAmount::from_atto(7);
    schedule_at(&streams, 0).unwrap();
    let allocation = allocate(&streams, 0, &reward);

    assert_eq!(TokenAmount::from_atto(3), allocation.miner);
    assert_eq!(vec![2, 3], allocation.portions.iter().map(|(id, _)| *id).collect::<Vec<_>>());
    assert_eq!(TokenAmount::from_atto(1), allocation.portions[0].1);
    assert_eq!(TokenAmount::zero(), allocation.portions[1].1);
    assert_eq!(TokenAmount::from_atto(3), allocation.burn);
    assert_eq!(
        reward,
        &allocation.miner
            + &allocation
                .portions
                .iter()
                .fold(TokenAmount::zero(), |sum, (_, amount)| sum + amount)
            + &allocation.burn
    );

    let mut accruals = vec![
        StreamAccrual { id: 2, amount: TokenAmount::from_atto(4) },
        StreamAccrual { id: 3, amount: TokenAmount::from_atto(5) },
    ];
    accrue(&mut accruals, &allocation.portions);
    assert_eq!(TokenAmount::from_atto(5), accruals[0].amount);
    assert_eq!(TokenAmount::from_atto(5), accruals[1].amount);

    // The row per explicit stream that the award credits is the accounting invariants.
    let state = StreamsState { streams, ..Default::default() };
    let error = accounting(&state, &accruals[..1]).unwrap_err();
    assert_eq!("explicit-stream accrual IDs do not match live explicit streams", error.to_string());
}

#[test]
fn indivisible_sentinel_portion_preserves_survivor_entitlements() {
    let third = DENOM / 3;
    let survivor_shares = shares(&[(101, third), (102, third)]);
    let reward = TokenAmount::from_atto(2);

    // Control: three ordinary recipients split the full two-atto explicit pool. Each recipient's
    // third floors to zero when claimed.
    let mut ordinary = StreamsState {
        streams: vec![stream(
            2,
            DENOM,
            Some(explicit(200, shares(&[(101, third), (102, third), (103, DENOM - 2 * third)]))),
        )],
        ..Default::default()
    };
    let ordinary_allocation = allocate(&ordinary.streams, 0, &reward);
    let mut ordinary_accruals = accruals_of(&ordinary_allocation.portions);
    let ordinary_claims =
        claim(&mut ordinary, &ordinary_accruals, 2, &[Address::new_id(101), Address::new_id(102)])
            .unwrap();
    assert_eq!(vec![TokenAmount::zero(), TokenAmount::zero()], ordinary_claims);

    // Sentinel case: the third recipient's share burns. Flooring the survivor pool yields one
    // atto, so the two surviving claims remain zero. Flooring the burn instead would leave a
    // two-atto pool and incorrectly pay each survivor one atto after denominator adjustment.
    let mut sentinel = StreamsState {
        streams: vec![stream(2, DENOM, Some(explicit(200, survivor_shares)))],
        ..Default::default()
    };
    let sentinel_allocation = allocate(&sentinel.streams, 0, &reward);
    assert_eq!(TokenAmount::from_atto(1), sentinel_allocation.portions[0].1);
    assert_eq!(TokenAmount::from_atto(1), sentinel_allocation.burn);
    assert_eq!(
        ordinary_allocation.portions[0].1,
        &sentinel_allocation.portions[0].1 + &sentinel_allocation.burn
    );

    let mut sentinel_accruals = accruals_of(&sentinel_allocation.portions);
    let sentinel_claims =
        claim(&mut sentinel, &sentinel_accruals, 2, &[Address::new_id(101), Address::new_id(102)])
            .unwrap();
    assert_eq!(ordinary_claims, sentinel_claims);

    accrue(&mut ordinary_accruals, &allocate(&ordinary.streams, 0, &reward).portions);
    accrue(&mut sentinel_accruals, &allocate(&sentinel.streams, 0, &reward).portions);
    let ordinary_claims =
        claim(&mut ordinary, &ordinary_accruals, 2, &[Address::new_id(101), Address::new_id(102)])
            .unwrap();
    let sentinel_claims =
        claim(&mut sentinel, &sentinel_accruals, 2, &[Address::new_id(101), Address::new_id(102)])
            .unwrap();
    assert_eq!(vec![TokenAmount::from_atto(1), TokenAmount::from_atto(1)], ordinary_claims);
    assert_eq!(ordinary_claims, sentinel_claims);
}

// The award splits nothing until the schedule holds at its own epoch, so these three are the
// states that leave it paying gas alone.
#[test]
fn invalid_weight_envelope_fails_the_award_schedule_check() {
    let streams = vec![
        stream(1, pct(60), None),
        stream(2, pct(50), Some(explicit(200, shares(&[(101, DENOM)])))),
    ];

    let error = schedule_at(&streams, 0).unwrap_err();
    assert_eq!("stream weights exceed DENOM at epoch 0: 1100000000000000000", error.to_string());
}

#[test]
fn malformed_implicit_weight_fails_the_award_schedule_check() {
    let mut malformed = stream(1, 0, None);
    malformed.weight.cap = DENOM + 1;
    let streams = vec![malformed, stream(2, pct(20), Some(explicit(200, shares(&[(101, DENOM)]))))];

    let error = schedule_at(&streams, 0).unwrap_err();
    assert_eq!("weight cap exceeds DENOM", error.to_string());
}

#[test]
fn malformed_explicit_weight_fails_the_award_schedule_check() {
    let mut malformed = stream(2, 0, Some(explicit(200, shares(&[(101, DENOM)]))));
    malformed.weight.cap = DENOM + 1;

    let error = schedule_at(&[malformed], 0).unwrap_err();
    assert_eq!("weight cap exceeds DENOM", error.to_string());
}
