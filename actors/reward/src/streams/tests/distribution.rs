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
    let normalized =
        normalize_shares(shares(&[(99, pct(20)), (101, pct(50)), (99, pct(30))])).unwrap();
    assert_eq!(shares(&[(101, pct(50))]), normalized);

    assert!(normalize_shares(shares(&[(99, 0), (101, DENOM)])).is_err());
    assert!(normalize_shares(shares(&[(99, pct(20)), (101, pct(40)), (101, pct(40)),])).is_err());
    assert!(normalize_shares(shares(&[(99, DENOM)])).unwrap().is_empty());

    let mut over_limit = shares(&[(101, DENOM - MAX_RECIPIENTS as u64)]);
    over_limit.extend(
        (0..MAX_RECIPIENTS).map(|_| RecipientShare { recipient: BURNT_FUNDS_ACTOR_ADDR, share: 1 }),
    );
    assert!(normalize_shares(over_limit).is_err());
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
