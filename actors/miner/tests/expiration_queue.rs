use fil_actor_miner::{ExpirationSet, PowerPair};
use fvm_ipld_bitfield::{BitField, MaybeBitField};
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
use std::convert::TryInto;
use std::iter::FromIterator;

fn mk_bitfield<const N: usize>(keys: [u64; N]) -> BitField {
    MaybeBitField::from_iter(keys.iter().copied()).try_into().unwrap()
}

fn on_time_sectors() -> BitField {
    mk_bitfield([5, 8, 9])
}
fn early_sectors() -> BitField {
    mk_bitfield([2, 3])
}
fn on_time_pledge() -> TokenAmount {
    TokenAmount::from(1_000)
}
fn active_power() -> PowerPair {
    PowerPair { raw: StoragePower::from(1 << 13), qa: StoragePower::from(1 << 14) }
}
fn faulty_power() -> PowerPair {
    PowerPair { raw: StoragePower::from(1 << 11), qa: StoragePower::from(1 << 12) }
}
fn default_set() -> ExpirationSet {
    let mut set = ExpirationSet::empty();
    set.add(
        &on_time_sectors(),
        &early_sectors(),
        &on_time_pledge(),
        &active_power(),
        &faulty_power(),
    )
    .unwrap();
    set
}
fn power_pair(factor: u64, shift: u64) -> PowerPair {
    PowerPair {
        raw: StoragePower::from(factor * (1 << shift)),
        qa: StoragePower::from(factor * (1 << (shift + 1))),
    }
}

#[test]
fn adds_sectors_and_power_to_empty_set() {
    let set = default_set();

    assert_eq!(set.on_time_sectors, on_time_sectors());
    assert_eq!(set.early_sectors, early_sectors());
    assert_eq!(set.on_time_pledge, on_time_pledge());
    assert_eq!(set.active_power, active_power());
    assert_eq!(set.faulty_power, faulty_power());

    assert_eq!(set.len(), 5);
}

#[test]
fn adds_sectors_and_power_to_non_empty_set() {
    let mut set = default_set();

    set.add(
        &mk_bitfield([6, 7, 11]),
        &mk_bitfield([1, 4]),
        &TokenAmount::from(300),
        &power_pair(3, 13),
        &power_pair(3, 11),
    )
    .unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([5, 6, 7, 8, 9, 11]));
    assert_eq!(set.early_sectors, mk_bitfield([1, 2, 3, 4]));
    assert_eq!(set.on_time_pledge, TokenAmount::from(1300));
    let active = power_pair(1, 15);
    assert_eq!(set.active_power, active);
    let faulty = power_pair(1, 13);
    assert_eq!(set.faulty_power, faulty);
}

#[test]
fn removes_sectors_and_power_set() {
    let mut set = default_set();

    set.remove(
        &mk_bitfield([9]),
        &mk_bitfield([2]),
        &TokenAmount::from(800),
        &power_pair(3, 11),
        &power_pair(3, 9),
    )
    .unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([5, 8]));
    assert_eq!(set.early_sectors, mk_bitfield([3]));
    assert_eq!(set.on_time_pledge, TokenAmount::from(200));
    let active = power_pair(1, 11);
    assert_eq!(set.active_power, active);
    let faulty = power_pair(1, 9);
    assert_eq!(set.faulty_power, faulty);
}

#[test]
fn remove_fails_when_pledge_underflows() {
    let mut set = default_set();

    let err = set
        .remove(
            &mk_bitfield([9]),
            &mk_bitfield([2]),
            &TokenAmount::from(1200),
            &power_pair(3, 11),
            &power_pair(3, 9),
        )
        .err()
        .unwrap();
    // XXX: This is not a good way to check for specific errors.
    assert!(err.to_string().contains("pledge underflow"));
}

#[test]
fn remove_fails_to_remove_sectors_it_does_not_contain() {
    let mut set = default_set();

    // remove unknown active sector 12
    let err = set
        .remove(
            &mk_bitfield([12]),
            &mk_bitfield([]),
            &TokenAmount::zero(),
            &power_pair(3, 11),
            &power_pair(3, 9),
        )
        .err()
        .unwrap();
    // XXX: This is not a good way to check for specific errors.
    assert!(err.to_string().contains("not contained"));

    // remove faulty sector 8, that is active in the set
    let err = set
        .remove(
            &mk_bitfield([0]),
            &mk_bitfield([8]),
            &TokenAmount::zero(),
            &power_pair(3, 11),
            &power_pair(3, 9),
        )
        .err()
        .unwrap();
    // XXX: This is not a good way to check for specific errors.
    assert!(err.to_string().contains("not contained"));
}

#[test]
fn remove_fails_when_active_or_fault_qa_power_underflows() {
    let mut set = default_set();

    // active removed power > active power
    let err = set
        .remove(
            &mk_bitfield([9]),
            &mk_bitfield([2]),
            &TokenAmount::from(200),
            &power_pair(3, 12),
            &power_pair(3, 9),
        )
        .err()
        .unwrap();
    // XXX: This is not a good way to check for specific errors.
    assert!(err.to_string().contains("power underflow"));

    let mut set = default_set();

    // faulty removed power > faulty power
    let err = set
        .remove(
            &mk_bitfield([9]),
            &mk_bitfield([2]),
            &TokenAmount::from(200),
            &power_pair(3, 11),
            &power_pair(3, 10),
        )
        .err()
        .unwrap();
    // XXX: This is not a good way to check for specific errors.
    assert!(err.to_string().contains("power underflow"));
}

#[test]
fn set_is_empty_when_all_sectors_removed() {
    let mut set = ExpirationSet::empty();

    assert!(set.is_empty());
    assert_eq!(set.len(), 0);

    set.add(
        &on_time_sectors(),
        &early_sectors(),
        &on_time_pledge(),
        &active_power(),
        &faulty_power(),
    )
    .unwrap();

    assert!(!set.is_empty());

    set.remove(
        &on_time_sectors(),
        &early_sectors(),
        &on_time_pledge(),
        &active_power(),
        &faulty_power(),
    )
    .unwrap();

    assert!(set.is_empty());
    assert_eq!(set.len(), 0);
}
