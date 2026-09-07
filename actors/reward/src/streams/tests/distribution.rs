use fil_actors_runtime::{BURNT_FUNDS_ACTOR_ADDR, REWARD_ACTOR_ADDR};
use fvm_shared::address::Address;

use super::*;
use crate::streams::distribution::{validate_shares, validate_stored_shares};

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

// A send from f02 to itself moves nothing and reports no error, so the reward actor is not a
// recipient in either form of the map.
#[test]
fn rejects_the_reward_actor_as_a_recipient() {
    let wire = vec![
        RecipientShare { recipient: REWARD_ACTOR_ADDR, share: DENOM / 2 },
        RecipientShare { recipient: Address::new_id(101), share: DENOM - DENOM / 2 },
    ];
    let error = validate_shares(&wire).unwrap_err();
    assert_eq!("the reward actor is not a share recipient", error.to_string());
    assert_eq!(error.to_string(), admit_shares(wire).unwrap_err().to_string());

    let stored = vec![RecipientShare { recipient: REWARD_ACTOR_ADDR, share: DENOM }];
    assert_eq!(
        "the reward actor is not a share recipient",
        validate_stored_shares(&stored).unwrap_err().to_string()
    );
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
fn installs_the_admitted_map_in_recipient_order() {
    let mut streams = base_state();

    set_shares(&mut streams, 2, shares(&[(103, pct(40)), (101, pct(60))])).unwrap();
    assert_eq!(
        shares(&[(101, pct(60)), (103, pct(40))]),
        streams.streams[1].distribution.as_ref().unwrap().shares
    );
}

#[test]
fn set_shares_rejects_a_missing_or_implicit_stream() {
    let mut streams = base_state();

    let error = set_shares(&mut streams.clone(), 9, shares(&[(101, DENOM)])).unwrap_err();
    assert_eq!("stream 9 not found", error.to_string());

    let error = set_shares(&mut streams, 1, shares(&[(101, DENOM)])).unwrap_err();
    assert_eq!("stream 1 is implicit", error.to_string());
}

#[test]
fn writer_replacement_keeps_the_share_map() {
    let mut streams = base_state();

    replace_writer(&mut streams, 2, Address::new_id(999)).unwrap();
    let distribution = streams.streams[1].distribution.as_ref().unwrap();
    assert_eq!(Address::new_id(999), distribution.writer);
    assert_eq!(shares(&[(101, DENOM)]), distribution.shares);

    assert!(replace_writer(&mut streams, 1, Address::new_id(999)).is_err());
    assert!(replace_writer(&mut streams, 9, Address::new_id(999)).is_err());
}

#[test]
fn removal_takes_the_stream_out_of_the_schedule() {
    let mut streams = base_state();

    remove_stream(&mut streams, 2).unwrap();
    assert_eq!(vec![1], streams.streams.iter().map(|stream| stream.id).collect::<Vec<_>>());
    assert!(remove_stream(&mut streams, 2).is_err());
}
