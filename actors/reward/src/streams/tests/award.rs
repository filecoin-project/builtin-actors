use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::*;

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
    // Stream 3's tenth of seven attos floors to nothing, so only stream 2 pays.
    assert_eq!(
        vec![(2, Address::new_id(101))],
        allocation
            .payouts
            .iter()
            .map(|payout| (payout.stream, payout.recipient))
            .collect::<Vec<_>>()
    );
    assert_eq!(TokenAmount::from_atto(1), payout(&allocation, 2, 101));
    assert_eq!(TokenAmount::zero(), payout(&allocation, 3, 102));
    assert_eq!(TokenAmount::from_atto(3), allocation.burn);
    assert_eq!(reward, &allocation.miner + &paid(&allocation) + &allocation.burn);
}

#[test]
fn splits_a_stream_portion_across_its_recipients_and_burns_the_dust() {
    let third = DENOM / 3;
    let streams = vec![
        stream(1, pct(50), None),
        stream(
            2,
            pct(50),
            Some(explicit(200, shares(&[(101, third), (102, third), (103, DENOM - 2 * third)]))),
        ),
    ];
    let reward = TokenAmount::from_atto(100);
    schedule_at(&streams, 0).unwrap();
    let allocation = allocate(&streams, 0, &reward);

    // Each of the three floors 50 * (1/3) to 16, so two attos of the portion are indivisible.
    assert_eq!(TokenAmount::from_atto(50), allocation.miner);
    assert_eq!(TokenAmount::from_atto(16), payout(&allocation, 2, 101));
    assert_eq!(TokenAmount::from_atto(16), payout(&allocation, 2, 102));
    assert_eq!(TokenAmount::from_atto(16), payout(&allocation, 2, 103));
    assert_eq!(TokenAmount::from_atto(2), allocation.burn);
    assert_eq!(reward, &allocation.miner + &paid(&allocation) + &allocation.burn);
}

#[test]
fn burns_the_whole_portion_of_an_all_sentinel_map() {
    let mut streams = base_state();
    set_shares(&mut streams, 2, shares(&[(99, DENOM)])).unwrap();
    assert!(streams.streams[1].distribution.as_ref().unwrap().shares.is_empty());

    let reward = TokenAmount::from_atto(100);
    let allocation = allocate(&streams.streams, 0, &reward);
    assert!(allocation.payouts.is_empty());
    assert_eq!(TokenAmount::from_atto(60), allocation.miner);
    assert_eq!(TokenAmount::from_atto(40), allocation.burn);
    assert_eq!(reward, &allocation.miner + &allocation.burn);
    assert_eq!(BURNT_FUNDS_ACTOR_ADDR, Address::new_id(99));
}

#[test]
fn pays_a_recipient_of_two_streams_once_for_each() {
    let shared_wallet = 101;
    let streams = vec![
        stream(1, pct(20), None),
        stream(2, pct(40), Some(explicit(200, shares(&[(shared_wallet, DENOM)])))),
        stream(
            3,
            pct(40),
            Some(explicit(201, shares(&[(shared_wallet, DENOM / 4), (102, DENOM - DENOM / 4)]))),
        ),
    ];
    let reward = TokenAmount::from_atto(100);
    schedule_at(&streams, 0).unwrap();
    let allocation = allocate(&streams, 0, &reward);

    assert_eq!(
        vec![2, 3, 3],
        allocation.payouts.iter().map(|payout| payout.stream).collect::<Vec<_>>()
    );
    assert_eq!(TokenAmount::from_atto(40), payout(&allocation, 2, shared_wallet));
    assert_eq!(TokenAmount::from_atto(10), payout(&allocation, 3, shared_wallet));
    assert_eq!(TokenAmount::from_atto(30), payout(&allocation, 3, 102));
    assert_eq!(TokenAmount::from_atto(20), allocation.miner);
    assert_eq!(TokenAmount::zero(), allocation.burn);
    assert_eq!(reward, &allocation.miner + &paid(&allocation) + &allocation.burn);
}

#[test]
fn pays_every_recipient_of_a_full_share_map() {
    let streams = vec![stream(2, DENOM, Some(explicit(200, full_share_map(1_000))))];
    let reward = TokenAmount::from_atto(3 * MAX_RECIPIENTS as i64);
    schedule_at(&streams, 0).unwrap();
    let allocation = allocate(&streams, 0, &reward);

    assert_eq!(MAX_RECIPIENTS, allocation.payouts.len());
    assert!(
        allocation.payouts.iter().all(|payout| payout.amount == TokenAmount::from_atto(3)),
        "each of the equal shares takes the same amount"
    );
    assert_eq!(reward, paid(&allocation));
    assert_eq!(TokenAmount::zero(), allocation.burn);
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

#[test]
fn pays_every_recipient_of_every_stream_at_the_configured_bound() {
    let table: Vec<Stream> = (0..MAX_STREAMS as u64)
        .map(|slot| {
            let id = 2 + slot;
            let first_recipient = 1_000 + slot * MAX_RECIPIENTS as u64;
            stream(id, pct(12), Some(explicit(200 + id, full_share_map(first_recipient))))
        })
        .collect();
    let streams = StreamsState { streams: table, ..Default::default() };

    // Eight streams at 12% of a 32000 atto reward: 3840 each, 60 to each of 64 recipients, and
    // the unassigned 4% burns.
    let reward = TokenAmount::from_atto(32_000);
    let (after, award) = full_award(&streams, 0, &reward);
    let allocation = award.allocation;

    assert_eq!(MAX_STREAMS * MAX_RECIPIENTS, allocation.payouts.len());
    let order: Vec<(StreamId, Address)> =
        allocation.payouts.iter().map(|payout| (payout.stream, payout.recipient)).collect();
    let mut ascending = order.clone();
    ascending.sort();
    assert_eq!(ascending, order, "payouts run in stream order, then recipient order");
    assert!(
        allocation.payouts.iter().all(|payout| payout.amount == TokenAmount::from_atto(60)),
        "each of the equal shares takes the same amount"
    );

    assert_eq!(TokenAmount::zero(), allocation.miner);
    assert_eq!(TokenAmount::from_atto(30_720), paid(&allocation));
    assert_eq!(TokenAmount::from_atto(1_280), allocation.burn);
    assert_eq!(streams, after);
}
