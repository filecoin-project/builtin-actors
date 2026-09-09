use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::*;
use crate::streams::distribution::validate_shares;

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
fn admits_and_strips_burn_sentinel_rows() {
    let normalized = admit_shares(shares(&[(99, pct(20)), (101, pct(50)), (99, pct(30))])).unwrap();
    assert_eq!(shares(&[(101, pct(50))]), normalized);

    assert!(admit_shares(shares(&[(99, 0), (101, DENOM)])).is_err());
    assert!(admit_shares(shares(&[(99, pct(20)), (101, pct(40)), (101, pct(40)),])).is_err());
    assert!(admit_shares(shares(&[(99, DENOM)])).unwrap().is_empty());

    let mut over_limit = shares(&[(101, DENOM - MAX_RECIPIENTS as u64)]);
    over_limit.extend(
        (0..MAX_RECIPIENTS).map(|_| RecipientShare { recipient: BURNT_FUNDS_ACTOR_ADDR, share: 1 }),
    );
    assert!(admit_shares(over_limit).is_err());
}

#[test]
fn folds_period_under_outgoing_shares_and_burns_only_dust() {
    let third = DENOM / 3;
    let old_shares = shares(&[(101, third), (102, third), (103, DENOM - 2 * third)]);
    let mut distribution = explicit(200, old_shares);
    distribution.payable.add(Address::new_id(104), TokenAmount::from_atto(5));
    distribution.claimed_period.add(Address::new_id(101), TokenAmount::from_atto(1));
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
        let before_explicit = supply.total_explicit.clone();
        let dust = set_shares(&mut streams, &mut accruals, 2, split.clone()).unwrap();
        assert_eq!(TokenAmount::from_atto(1), dust);
        supply.burn_dust(dust);
        assert_eq!(before_burn, supply.total_burn);
        assert_eq!(before_explicit, supply.total_explicit);
        supply.assert_invariants(&streams, &accruals);
    }

    assert_eq!(TokenAmount::from_atto(22), supply.total_burn);
    assert_eq!(TokenAmount::from_atto(20), supply.total_explicit);
    assert_eq!(TokenAmount::from_atto(2), supply.total_dust);
    supply.assert_invariants(&streams, &accruals);
}

#[test]
fn replace_address_renames_recipient_and_leaves_balances_in_place() {
    let mut distribution = explicit(200, shares(&[(101, DENOM / 2), (102, DENOM - DENOM / 2)]));
    distribution.payable.add(Address::new_id(101), TokenAmount::from_atto(5));
    distribution.claimed_period.add(Address::new_id(101), TokenAmount::from_atto(1));
    let mut streams = StreamsState {
        streams: vec![stream(2, pct(20), Some(distribution))],
        ..Default::default()
    };
    let mut accruals = vec![StreamAccrual { id: 2, amount: TokenAmount::from_atto(10) }];

    let burn =
        replace_address(&mut streams, &mut accruals, 2, Address::new_id(101), Address::new_id(103))
            .unwrap();

    let distribution = streams.streams[0].distribution.as_ref().unwrap();
    assert!(burn.is_zero());
    assert_eq!(TokenAmount::zero(), accruals[0].amount);
    assert!(distribution.claimed_period.is_empty());
    assert_eq!(shares(&[(102, DENOM - DENOM / 2), (103, DENOM / 2)]), distribution.shares);
    // 101 holds its carried 5 plus its 5 earned this period less 1 already claimed; 103 takes the
    // future share and starts from zero.
    assert_eq!(TokenAmount::from_atto(9), amount(&distribution.payable, 101));
    assert_eq!(TokenAmount::zero(), amount(&distribution.payable, 103));
    assert_eq!(TokenAmount::from_atto(5), amount(&distribution.payable, 102));
}

#[test]
fn replace_address_matches_setshares_with_the_recipient_row_renamed() {
    let build = || {
        let mut distribution = explicit(200, shares(&[(101, pct(30)), (102, pct(70))]));
        distribution.payable.add(Address::new_id(101), TokenAmount::from_atto(7));
        distribution.claimed_period.add(Address::new_id(102), TokenAmount::from_atto(2));
        (
            StreamsState {
                streams: vec![stream(2, pct(20), Some(distribution))],
                ..Default::default()
            },
            vec![StreamAccrual { id: 2, amount: TokenAmount::from_atto(13) }],
        )
    };

    let (mut swapped, mut swapped_accruals) = build();
    let swapped_burn = replace_address(
        &mut swapped,
        &mut swapped_accruals,
        2,
        Address::new_id(101),
        Address::new_id(103),
    )
    .unwrap();

    let (mut rewritten, mut rewritten_accruals) = build();
    let rewritten_burn = set_shares(
        &mut rewritten,
        &mut rewritten_accruals,
        2,
        shares(&[(103, pct(30)), (102, pct(70))]),
    )
    .unwrap();

    assert_eq!(rewritten_burn, swapped_burn);
    assert_eq!(rewritten_accruals, swapped_accruals);
    assert_eq!(rewritten, swapped);

    let distribution = swapped.streams[0].distribution.as_ref().unwrap();
    assert_eq!(shares(&[(102, pct(70)), (103, pct(30))]), distribution.shares);
    // 101 keeps its carried 7 plus the 3 it earned under the outgoing map.
    assert_eq!(TokenAmount::from_atto(7 + 3), amount(&distribution.payable, 101));
}

#[test]
fn replace_address_keeps_the_new_address_payable_row_apart_from_the_old_one() {
    let mut distribution = explicit(200, shares(&[(101, DENOM / 2), (102, DENOM - DENOM / 2)]));
    distribution.payable.add(Address::new_id(101), TokenAmount::from_atto(4));
    distribution.payable.add(Address::new_id(103), TokenAmount::from_atto(6));
    let mut streams = StreamsState {
        streams: vec![stream(2, pct(20), Some(distribution))],
        ..Default::default()
    };
    let mut accruals = vec![StreamAccrual { id: 2, amount: TokenAmount::from_atto(10) }];

    replace_address(&mut streams, &mut accruals, 2, Address::new_id(101), Address::new_id(103))
        .unwrap();

    let distribution = streams.streams[0].distribution.as_ref().unwrap();
    assert_eq!(shares(&[(102, DENOM - DENOM / 2), (103, DENOM / 2)]), distribution.shares);
    assert_eq!(TokenAmount::from_atto(9), amount(&distribution.payable, 101));
    assert_eq!(TokenAmount::from_atto(6), amount(&distribution.payable, 103));
    assert_eq!(TokenAmount::from_atto(5), amount(&distribution.payable, 102));
}

#[test]
fn replace_address_rejects_missing_or_colliding_recipients() {
    let (mut streams, mut accruals) = base_state();
    streams.streams[1].distribution.as_mut().unwrap().shares =
        shares(&[(101, DENOM / 2), (102, DENOM - DENOM / 2)]);

    assert!(
        replace_address(&mut streams, &mut accruals, 2, Address::new_id(109), Address::new_id(103))
            .is_err()
    );
    assert!(
        replace_address(&mut streams, &mut accruals, 2, Address::new_id(101), Address::new_id(102))
            .is_err()
    );
    // The old address is already the new one.
    assert!(
        replace_address(&mut streams, &mut accruals, 2, Address::new_id(101), Address::new_id(101))
            .is_err()
    );
    // f099 is never stored, so the old-recipient check refuses it.
    assert!(
        replace_address(
            &mut streams,
            &mut accruals,
            2,
            BURNT_FUNDS_ACTOR_ADDR,
            Address::new_id(103)
        )
        .is_err()
    );
    // Implicit stream.
    assert!(
        replace_address(&mut streams, &mut accruals, 1, Address::new_id(101), Address::new_id(103))
            .is_err()
    );
}

#[test]
fn replace_address_with_burn_sentinel_matches_setshares_with_an_f099_row() {
    let build = || {
        let mut distribution = explicit(200, shares(&[(101, pct(30)), (102, pct(70))]));
        distribution.payable.add(Address::new_id(101), TokenAmount::from_atto(7));
        distribution.claimed_period.add(Address::new_id(102), TokenAmount::from_atto(2));
        (
            StreamsState {
                streams: vec![stream(2, pct(20), Some(distribution))],
                ..Default::default()
            },
            vec![StreamAccrual { id: 2, amount: TokenAmount::from_atto(13) }],
        )
    };

    let (mut swapped, mut swapped_accruals) = build();
    let swapped_burn = replace_address(
        &mut swapped,
        &mut swapped_accruals,
        2,
        Address::new_id(101),
        BURNT_FUNDS_ACTOR_ADDR,
    )
    .unwrap();

    let (mut rewritten, mut rewritten_accruals) = build();
    let rewritten_burn = set_shares(
        &mut rewritten,
        &mut rewritten_accruals,
        2,
        shares(&[(99, pct(30)), (102, pct(70))]),
    )
    .unwrap();

    assert_eq!(rewritten_burn, swapped_burn);
    assert_eq!(rewritten_accruals, swapped_accruals);
    assert_eq!(rewritten, swapped);

    let distribution = swapped.streams[0].distribution.as_ref().unwrap();
    assert_eq!(shares(&[(102, pct(70))]), distribution.shares);
    // 101 keeps its carried 7 plus the 3 it earned before the row was dropped.
    assert_eq!(TokenAmount::from_atto(7 + 3), amount(&distribution.payable, 101));
}

#[test]
fn replace_address_caps_the_payable_union_like_set_shares() {
    let current = full_share_map(100);
    let mut distribution = explicit(300, current.clone());
    distribution.payable = (0..MAX_RECIPIENTS)
        .map(|offset| RecipientAmount {
            recipient: Address::new_id(1_000 + offset as u64),
            amount: TokenAmount::from_atto(1),
        })
        .collect::<Vec<_>>()
        .into();
    let mut streams = StreamsState {
        streams: vec![stream(2, pct(20), Some(distribution))],
        ..Default::default()
    };
    // One atto per recipient, so the fold gives every stored recipient a payable row of its own.
    let mut accruals =
        vec![StreamAccrual { id: 2, amount: TokenAmount::from_atto(MAX_RECIPIENTS as u64) }];

    // The fold leaves 128 payable rows, so only an address already holding one fits.
    let error = replace_address(
        &mut streams,
        &mut accruals,
        2,
        current[0].recipient,
        Address::new_id(2_000),
    )
    .unwrap_err();
    assert_eq!(
        format!(
            "stream 2 payable row reservation {} exceeds maximum {MAX_PAYABLE_ROWS_PER_STREAM}",
            MAX_PAYABLE_ROWS_PER_STREAM + 1
        ),
        error.to_string()
    );

    // A former recipient already holds a row, so the union stands at the cap.
    replace_address(&mut streams, &mut accruals, 2, current[0].recipient, Address::new_id(1_000))
        .unwrap();
    let distribution = streams.streams[0].distribution.as_ref().unwrap();
    assert_eq!(MAX_PAYABLE_ROWS_PER_STREAM, distribution.payable.union_len(&distribution.shares));
}

#[test]
fn replace_address_recharges_a_pending_removal_reservation() {
    let (mut streams, mut accruals) = base_state();
    streams.streams[1].distribution.as_mut().unwrap().shares = full_share_map(100);
    streams.tombstones = vec![tombstone(3, 1_000, MAX_TOMBSTONE_ROWS - MAX_RECIPIENTS)];
    accruals[0].amount = TokenAmount::from_atto(MAX_RECIPIENTS as u64);
    queue_remove_stream(&mut streams, &accruals, 0, 1, 2).unwrap();

    // The removal reserved 64 rows against the map it will fold, and the replace adds a 65th. The
    // actor's transaction discards the rejected ledger; the harness here does the same.
    let error = replace_address(
        &mut streams,
        &mut accruals,
        2,
        Address::new_id(100),
        Address::new_id(2_000),
    )
    .unwrap_err();
    assert!(error.to_string().contains("tombstone row reservation 257"), "{error}");
}

#[test]
fn claims_live_and_payable_amounts_once_in_request_order() {
    let mut distribution = explicit(200, shares(&[(101, DENOM / 2), (102, DENOM - DENOM / 2)]));
    distribution.payable = vec![
        RecipientAmount { recipient: Address::new_id(101), amount: TokenAmount::from_atto(3) },
        RecipientAmount { recipient: Address::new_id(102), amount: TokenAmount::from_atto(4) },
    ]
    .into();
    distribution.claimed_period = vec![RecipientAmount {
        recipient: Address::new_id(101),
        amount: TokenAmount::from_atto(2),
    }]
    .into();
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
            ]
            .into(),
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
