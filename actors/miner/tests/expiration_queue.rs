use fil_actor_miner::{
    power_for_sectors, ExpirationQueue, ExpirationSet, PowerPair, SectorOnChainInfo,
};
use fil_actors_runtime::{
    test_utils::{make_sealed_cid, MockRuntime},
    DealWeight,
};
use fvm_ipld_amt::Amt;
use fvm_ipld_bitfield::{BitField, MaybeBitField};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::{ChainEpoch, QuantSpec, NO_QUANTIZATION};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{SectorNumber, SectorSize, StoragePower};
use std::convert::TryInto;
use std::iter::FromIterator;

mod util;
use util::*;

const TEST_AMT_BITWIDTH: u32 = 3;

fn mk_bitfield<const N: usize>(keys: [u64; N]) -> BitField {
    MaybeBitField::from_iter(keys.iter().copied()).try_into().unwrap()
}

fn on_time_sectors() -> BitField {
    mk_bitfield([5, 8, 9])
}
fn early_sectors() -> BitField {
    mk_bitfield([101, 102])
}
fn faulty_sectors() -> BitField {
    mk_bitfield([2, 3])
}
fn on_time_pledge() -> TokenAmount {
    TokenAmount::from_atto(1_000)
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
        &faulty_sectors(),
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
    assert_eq!(set.proof_expiring_sectors, early_sectors());
    assert_eq!(set.faulty_sectors, faulty_sectors());
    assert_eq!(set.on_time_pledge, on_time_pledge());
    assert_eq!(set.active_power, active_power());
    assert_eq!(set.faulty_power, faulty_power());

    assert_eq!(set.len(), 7);
}

#[test]
fn adds_sectors_and_power_to_non_empty_set() {
    let mut set = default_set();

    set.add(
        &mk_bitfield([6, 7, 11]),
        &mk_bitfield([103, 104]),
        &mk_bitfield([1, 4]),
        &TokenAmount::from_atto(300),
        &power_pair(3, 13),
        &power_pair(3, 11),
    )
    .unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([5, 6, 7, 8, 9, 11]));
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([101, 102, 103, 104]));
    assert_eq!(set.faulty_sectors, mk_bitfield([1, 2, 3, 4]));
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(1300));
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
        &mk_bitfield([101]),
        &mk_bitfield([2]),
        &TokenAmount::from_atto(800),
        &power_pair(3, 11),
        &power_pair(3, 9),
    )
    .unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([5, 8]));
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([102]));
    assert_eq!(set.faulty_sectors, mk_bitfield([3]));
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(200));
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
            &mk_bitfield([101]),
            &mk_bitfield([2]),
            &TokenAmount::from_atto(1200),
            &power_pair(3, 11),
            &power_pair(3, 9),
        )
        .err()
        .unwrap();
    // XXX: This is not a good way to check for specific errors.
    //      See: https://github.com/filecoin-project/builtin-actors/issues/338
    assert!(err.to_string().contains("pledge underflow"));
}

#[test]
fn remove_fails_to_remove_sectors_it_does_not_contain() {
    let mut set = default_set();

    // remove unknown active sector 12
    let err = set
        .remove(
            &mk_bitfield([12]),
            &BitField::new(),
            &mk_bitfield([]),
            &TokenAmount::zero(),
            &power_pair(3, 11),
            &power_pair(3, 9),
        )
        .err()
        .unwrap();
    // XXX: This is not a good way to check for specific errors.
    //      See: https://github.com/filecoin-project/builtin-actors/issues/338
    assert!(err.to_string().contains("not contained"));

    // remove faulty sector 8, that is active in the set
    let err = set
        .remove(
            &mk_bitfield([0]),
            &BitField::new(),
            &mk_bitfield([8]),
            &TokenAmount::zero(),
            &power_pair(3, 11),
            &power_pair(3, 9),
        )
        .err()
        .unwrap();
    // XXX: This is not a good way to check for specific errors.
    //      See: https://github.com/filecoin-project/builtin-actors/issues/338
    assert!(err.to_string().contains("not contained"));

    // remove early sector 8, it is in active set,
    let err = set
        .remove(
            &mk_bitfield([0]),
            &mk_bitfield([8]),
            &BitField::new(),
            &TokenAmount::zero(),
            &power_pair(3, 11),
            &power_pair(3, 9),
        )
        .err()
        .unwrap();
    // XXX: This is not a good way to check for specific errors.
    //      See: https://github.com/filecoin-project/builtin-actors/issues/338
    assert!(err.to_string().contains("not contained"));
}

#[test]
fn remove_fails_when_active_or_fault_qa_power_underflows() {
    let mut set = default_set();

    // active removed power > active power
    let err = set
        .remove(
            &mk_bitfield([9]),
            &BitField::new(),
            &mk_bitfield([2]),
            &TokenAmount::from_atto(200),
            &power_pair(3, 12),
            &power_pair(3, 9),
        )
        .err()
        .unwrap();
    // XXX: This is not a good way to check for specific errors.
    //      See: https://github.com/filecoin-project/builtin-actors/issues/338
    assert!(err.to_string().contains("power underflow"));

    let mut set = default_set();

    // faulty removed power > faulty power
    let err = set
        .remove(
            &mk_bitfield([9]),
            &BitField::new(),
            &mk_bitfield([2]),
            &TokenAmount::from_atto(200),
            &power_pair(3, 11),
            &power_pair(3, 10),
        )
        .err()
        .unwrap();
    // XXX: This is not a good way to check for specific errors.
    //      See: https://github.com/filecoin-project/builtin-actors/issues/338
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
        &faulty_sectors(),
        &on_time_pledge(),
        &active_power(),
        &faulty_power(),
    )
    .unwrap();

    assert!(!set.is_empty());

    set.remove(
        &on_time_sectors(),
        &early_sectors(),
        &faulty_sectors(),
        &on_time_pledge(),
        &active_power(),
        &faulty_power(),
    )
    .unwrap();

    assert!(set.is_empty());
    assert_eq!(set.len(), 0);
}

fn sectors() -> [SectorOnChainInfo; 8] {
    [
        test_sector(2, 100, 1, 50, 60, 1000),
        test_sector(3, 100, 2, 51, 61, 1001),
        test_sector(7, 100, 3, 52, 62, 1002),
        test_sector(8, 100, 4, 53, 63, 1003),
        test_sector(11, 100, 5, 54, 64, 1004),
        test_sector(13, 100, 6, 55, 65, 1005),
        test_sector(20, 9, 7, 56, 66, 1006),
        test_sector(20, 14, 8, 57, 67, 1007),
    ]
}

const SECTOR_SIZE: SectorSize = SectorSize::_32GiB;

#[test]
fn added_sectors_can_be_popped_off_queue() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();

    let mut queue = empty_expiration_queue(&rt);
    let (sec_nums, power, pledge) = queue.add_active_sectors(&sectors(), SECTOR_SIZE).unwrap();
    assert_eq!(sec_nums, mk_bitfield([1, 2, 3, 4, 5, 6, 7, 8]));
    assert_eq!(power, power_for_sectors(SECTOR_SIZE, &sectors()));
    assert_eq!(pledge, sectors().iter().map(|s| &s.initial_pledge).sum());

    // default test quantizing of 1 means every sector is in its own expriation set
    assert_eq!(sectors().len(), queue.amt.count() as usize);

    // pop off sectors up to and including epoch 7
    let set = queue.pop_until(7).unwrap();

    // only 5 sectors remain
    assert_eq!(queue.amt.count(), 5);

    assert_eq!(set.on_time_sectors, mk_bitfield([1, 2, 3]));
    assert!(set.proof_expiring_sectors.is_empty());
    assert!(set.faulty_sectors.is_empty());

    let active_power = power_for_sectors(SECTOR_SIZE, &sectors()[0..3]);
    let faulty_power = PowerPair::zero();

    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(3003));
    assert_eq!(set.active_power, active_power);
    assert_eq!(set.faulty_power, faulty_power);

    // pop off sectors up to and including epoch 14
    let set = queue.pop_until(9).unwrap();

    // only 3 sectors remain
    assert_eq!(queue.amt.count(), 3);

    assert_eq!(set.on_time_sectors, mk_bitfield([4]));
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([7]));
    assert!(set.faulty_sectors.is_empty());

    let active_power =
        power_for_sectors(SECTOR_SIZE, &[sectors()[3].clone(), sectors()[6].clone()]);
    let faulty_power = PowerPair::zero();

    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(1003));
    assert_eq!(set.active_power, active_power);
    assert_eq!(set.faulty_power, faulty_power);

    // pop off rest up to and including epoch 20
    let set = queue.pop_until(20).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([5, 6]));
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([8]));
    assert!(set.faulty_sectors.is_empty());

    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(2009)); // sum of last 3 sector pledges
    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors()[0..3]));
    assert_eq!(set.faulty_power, PowerPair::zero());

    // queue is now empty
    assert_eq!(queue.amt.count(), 0);
}

#[test]
fn quantizes_added_sectors_by_expiration() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();

    let mut queue = empty_expiration_queue_with_quantizing(&rt, QuantSpec { unit: 5, offset: 3 });

    let (sec_nums, power, pledge) = queue.add_active_sectors(&sectors(), SECTOR_SIZE).unwrap();
    assert_eq!(sec_nums, mk_bitfield([1, 2, 3, 4, 5, 6, 7, 8]));
    assert_eq!(power, power_for_sectors(SECTOR_SIZE, &sectors()));
    assert_eq!(pledge, sectors().iter().map(|s| &s.initial_pledge).sum());

    // quantizing spec means sectors should be grouped into 4 sets expiring at 3, 8, 13 and 18
    assert_eq!(queue.amt.count(), 4);

    // set popped before first quantized sector should be empty
    let set = queue.pop_until(2).unwrap();
    assert!(set.on_time_sectors.is_empty());
    assert!(set.proof_expiring_sectors.is_empty());
    assert_eq!(queue.amt.count(), 4);

    // first 2 sectors will be in first set popped off at quantization offset (3)
    let set = queue.pop_until(3).unwrap();
    assert_eq!(set.on_time_sectors, mk_bitfield([1, 2]));
    assert!(set.proof_expiring_sectors.is_empty());
    assert_eq!(queue.amt.count(), 3);

    let _ = queue.amt.flush().unwrap();

    // no sectors will be popped off in quantization interval
    let set = queue.pop_until(7).unwrap();
    assert!(set.on_time_sectors.is_empty());
    assert!(set.proof_expiring_sectors.is_empty());
    assert_eq!(queue.amt.count(), 3);

    // next 2 sectors will be in first set popped off after quantization interval (8)
    let set = queue.pop_until(8).unwrap();
    assert_eq!(set.on_time_sectors, mk_bitfield([3, 4]));
    assert!(set.proof_expiring_sectors.is_empty());
    assert_eq!(queue.amt.count(), 2);

    let _ = queue.amt.flush().unwrap();

    // no sectors will be popped off in quantization interval
    let set = queue.pop_until(12).unwrap();
    assert!(set.on_time_sectors.is_empty());
    assert!(set.proof_expiring_sectors.is_empty());
    assert_eq!(queue.amt.count(), 2);

    // rest of sectors will be in first set popped off after quantization interval (13)
    let set = queue.pop_until(13).unwrap();
    assert_eq!(set.on_time_sectors, mk_bitfield([5, 6]));
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([7]));
    assert_eq!(queue.amt.count(), 1);

    let set = queue.pop_until(18).unwrap();
    assert!(set.on_time_sectors.is_empty());
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([8]));
    assert_eq!(queue.amt.count(), 0);
}

#[test]
fn reschedules_sectors_as_faults() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();

    // Create 3 expiration sets with 2 sectors apiece
    let mut queue = empty_expiration_queue_with_quantizing(&rt, QuantSpec { unit: 4, offset: 1 });
    let (_sec_nums, _power, _pledge) = queue.add_active_sectors(&sectors(), SECTOR_SIZE).unwrap();

    let _ = queue.amt.flush().unwrap();

    // Fault middle sectors to expire at epoch 6
    // This faults one sector from the first set, all of the second set and one from the third.
    // Faulting at epoch 6 means the first 2 will expire on time, but the last will be early and
    // moved to the second set
    let faulted = &[&sectors()[1..5], &sectors()[6..7]].concat();
    let power_delta = queue.reschedule_as_faults(6, faulted, SECTOR_SIZE).unwrap();
    assert_eq!(power_delta, power_for_sectors(SECTOR_SIZE, faulted));

    let _ = queue.amt.flush().unwrap();

    // expect first set to contain first two sectors but with the seconds power moved to faulty power
    require_no_expiration_groups_before(5, &mut queue);
    let set = queue.pop_until(5).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([1, 2]));
    assert!(set.faulty_sectors.is_empty());

    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(2001));
    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors()[0..1]));
    assert_eq!(set.faulty_power, power_for_sectors(SECTOR_SIZE, &sectors()[1..2]));

    // expect the second set to have all faulty power and now contain 5th sector as an early sector
    require_no_expiration_groups_before(9, &mut queue);
    let set = queue.pop_until(9).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([3, 4]));
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([7]));
    assert_eq!(set.faulty_sectors, mk_bitfield([5]));

    // pledge is kept from original 2 sectors. Pledge from new early sector is NOT added.
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(2005));

    assert_eq!(set.active_power, PowerPair::zero());
    assert_eq!(
        set.faulty_power,
        power_for_sectors(SECTOR_SIZE, &[&sectors()[2..5], &sectors()[6..7]].concat())
    );

    // expect second to last set to only contain non faulty sector
    require_no_expiration_groups_before(13, &mut queue);
    let set = queue.pop_until(13).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([6]));
    assert!(set.proof_expiring_sectors.is_empty());
    assert!(set.faulty_sectors.is_empty());

    // Pledge from sector moved from this set is dropped
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(1005));

    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors()[5..6]));
    assert_eq!(set.faulty_power, PowerPair::zero());

    // expect last set to only contain early sector
    require_no_expiration_groups_before(14, &mut queue);
    let set = queue.pop_until(20).unwrap();

    assert!(set.on_time_sectors.is_empty());
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([8]));
    assert!(set.faulty_sectors.is_empty());

    // Pledge from sector moved from this set is dropped
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(0));

    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors()[7..8]));
    assert_eq!(set.faulty_power, PowerPair::zero());
}

#[test]
fn reschedules_all_sectors_as_faults() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();

    // Create expiration 3 sets with 2 sectors apiece
    let mut queue = empty_expiration_queue_with_quantizing(&rt, QuantSpec { unit: 4, offset: 1 });
    let (_sec_nums, _power, _pledge) = queue.add_active_sectors(&sectors(), SECTOR_SIZE).unwrap();

    let _ = queue.amt.flush().unwrap();

    queue
        .amt
        .for_each(|c, v| {
            println!("{}: {:?}", c, v);
            Ok(())
        })
        .unwrap();

    // Fault all sectors
    // This converts the first 2 sets to faults and adds the 3rd set as early sectors to the second set
    queue.reschedule_all_as_faults(6).unwrap();

    let _ = queue.amt.flush().unwrap();
    queue
        .amt
        .for_each(|c, v| {
            println!("{}: {:?}", c, v);
            Ok(())
        })
        .unwrap();

    // expect first set to contain first two sectors but with all power moved to faulty power
    require_no_expiration_groups_before(5, &mut queue);
    let set = queue.pop_until(5).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([1, 2])); // sectors are unmoved
    assert!(set.proof_expiring_sectors.is_empty());
    assert!(set.faulty_sectors.is_empty());

    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(2001)); // pledge is same

    // active power is converted to fault power
    assert_eq!(set.active_power, PowerPair::zero());
    assert_eq!(set.faulty_power, power_for_sectors(SECTOR_SIZE, &sectors()[0..2]));

    // expect the second set to have all faulty power and now contain 5th and 6th sectors as an early sectors
    require_no_expiration_groups_before(9, &mut queue);
    let set = queue.pop_until(9).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([3, 4]));
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([7]));
    assert_eq!(set.faulty_sectors, mk_bitfield([5, 6, 8]));

    // pledge is kept from original 2 sectors. Pledge from new early sectors is NOT added.
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(2005));

    // fault power is all power for sectors previously in the second and third sets
    assert_eq!(set.active_power, PowerPair::zero());
    assert_eq!(set.faulty_power, power_for_sectors(SECTOR_SIZE, &sectors()[2..]));

    // expect last set to only contain non faulty sector
    require_no_expiration_groups_before(13, &mut queue);
    let set = queue.pop_until(13).unwrap();

    assert!(set.on_time_sectors.is_empty());
    assert!(set.faulty_sectors.is_empty());

    // all pledge is dropped
    assert!(set.on_time_pledge.is_zero());

    assert_eq!(set.active_power, PowerPair::zero());
    assert_eq!(set.faulty_power, PowerPair::zero());
}

#[test]
fn reschedule_recover_restores_all_sector_stats() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();

    // Create expiration 3 sets with 2 sectors apiece
    let mut queue = empty_expiration_queue_with_quantizing(&rt, QuantSpec { unit: 4, offset: 1 });
    let (_sec_nums, _power, _pledge) = queue.add_active_sectors(&sectors(), SECTOR_SIZE).unwrap();

    let _ = queue.amt.flush().unwrap();

    // Fault middle sectors to expire at epoch 6 to put sectors in a state
    // described in "reschedules sectors as faults"
    let faults = [&sectors()[1..5], &sectors()[6..7]].concat();
    let _ = queue.reschedule_as_faults(6, &faults, SECTOR_SIZE).unwrap();

    let _ = queue.amt.flush().unwrap();

    // mark faulted sectors as recovered
    let recovered = queue.reschedule_recovered(&faults, SECTOR_SIZE).unwrap();
    assert_eq!(recovered, power_for_sectors(SECTOR_SIZE, &faults));

    // expect first set to contain first two sectors with active power
    require_no_expiration_groups_before(5, &mut queue);
    let set = queue.pop_until(5).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([1, 2]));
    assert!(set.proof_expiring_sectors.is_empty());
    assert!(set.faulty_sectors.is_empty());

    // pledge from both sectors
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(2001));

    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors()[0..2]));
    assert_eq!(set.faulty_power, PowerPair::zero());

    // expect second set to have lost early sector 5 and have active power just from 3 and 4
    require_no_expiration_groups_before(9, &mut queue);
    let set = queue.pop_until(9).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([3, 4]));
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([7]));
    assert!(set.faulty_sectors.is_empty());

    // pledge is kept from original 2 sectors
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(2005));

    assert_eq!(
        set.active_power,
        power_for_sectors(SECTOR_SIZE, &[&sectors()[2..4], &sectors()[6..7]].concat())
    );
    assert_eq!(set.faulty_power, PowerPair::zero());

    // expect sector 5 to be returned to last setu
    require_no_expiration_groups_before(13, &mut queue);
    let set = queue.pop_until(20).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([5, 6]));
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([8]));
    assert!(set.faulty_sectors.is_empty());

    // Pledge from sector 5 is restored
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(2009));

    assert_eq!(
        set.active_power,
        power_for_sectors(SECTOR_SIZE, &[&sectors()[4..6], &sectors()[7..8]].concat())
    );
    assert_eq!(set.faulty_power, PowerPair::zero());
}

#[test]
fn replaces_sectors_with_new_sectors() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();

    // Create expiration 3 sets
    let mut queue = empty_expiration_queue_with_quantizing(&rt, QuantSpec { unit: 4, offset: 1 });

    // add sectors to each set
    let sectors = sectors();
    let (_sec_nums, _power, _pledge) = queue
        .add_active_sectors(
            &[
                sectors[0].clone(),
                sectors[1].clone(),
                sectors[3].clone(),
                sectors[5].clone(),
                sectors[6].clone(),
            ],
            SECTOR_SIZE,
        )
        .unwrap();

    let _ = queue.amt.flush().unwrap();

    // remove all from first set, replace second set, and append to third
    let to_remove =
        [sectors[0].clone(), sectors[1].clone(), sectors[3].clone(), sectors[6].clone()];
    let to_add = [sectors[2].clone(), sectors[4].clone(), sectors[7].clone()];
    let (removed, added, power_delta, pledge_delta) =
        queue.replace_sectors(&to_remove, &to_add, SECTOR_SIZE).unwrap();
    assert_eq!(removed, mk_bitfield([1, 2, 4, 7]));
    assert_eq!(added, mk_bitfield([3, 5, 8]));
    let added_power = power_for_sectors(SECTOR_SIZE, &to_add);
    assert_eq!(power_delta, &added_power - &power_for_sectors(SECTOR_SIZE, &to_remove));
    assert_eq!(
        TokenAmount::from_atto(1002 + 1004 - 1000 - 1001 - 1003 - 1006 + 1007),
        pledge_delta
    );

    // first set is gone
    require_no_expiration_groups_before(9, &mut queue);

    // second set is replaced
    let set = queue.pop_until(9).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([3]));
    assert!(set.faulty_sectors.is_empty());

    // pledge and power is only from sector 3
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(1002));
    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors[2..3]));
    assert_eq!(set.faulty_power, PowerPair::zero());

    // last set appends sector 6
    require_no_expiration_groups_before(13, &mut queue);
    let set = queue.pop_until(13).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([5, 6]));
    assert!(set.faulty_sectors.is_empty());

    // pledge and power are the sum of old and new sectors
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(2009));
    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors[4..6]));
    assert_eq!(set.faulty_power, PowerPair::zero());

    let set = queue.pop_until(20).unwrap();

    assert!(set.on_time_sectors.is_empty());
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([8]));
    assert!(set.faulty_sectors.is_empty());

    // pledge and power are the sum of old and new sectors
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(0));
    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors[7..]));
    assert_eq!(set.faulty_power, PowerPair::zero());
}

#[test]
fn removes_sectors() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();

    // add all sectors into 4 sets
    let mut queue = empty_expiration_queue_with_quantizing(&rt, QuantSpec { unit: 4, offset: 1 });
    let _ = queue.add_active_sectors(&sectors(), SECTOR_SIZE).unwrap();

    let _ = queue.amt.flush().unwrap();
    assert_eq!(queue.amt.count(), 4);

    // put queue in a state where some sectors are early and some are faulty
    let _ = queue.reschedule_as_faults(6, &sectors()[1..], SECTOR_SIZE).unwrap();

    let _ = queue.amt.flush().unwrap();

    // remove an active sector from first set, faulty sector and early faulty sector from second set,
    let to_remove = [
        sectors()[0].clone(),
        sectors()[3].clone(),
        sectors()[4].clone(),
        sectors()[5].clone(),
        sectors()[6].clone(),
        sectors()[7].clone(),
    ];

    // and only sector from last set
    let faults = mk_bitfield([4, 5, 6, 7, 8]);

    // label the last as recovering
    let recovering = mk_bitfield([6, 8]);
    let (removed, recovering_power) =
        queue.remove_sectors(&rt.policy, &to_remove, &faults, &recovering, SECTOR_SIZE).unwrap();

    // assert all return values are correct
    assert_eq!(removed.on_time_sectors, mk_bitfield([1, 4]));
    assert_eq!(removed.faulty_sectors, mk_bitfield([5, 6, 8]));
    assert_eq!(removed.proof_expiring_sectors, mk_bitfield([7]));
    assert_eq!(removed.on_time_pledge, TokenAmount::from_atto(1000 + 1003)); // only on-time
    assert_eq!(removed.active_power, power_for_sectors(SECTOR_SIZE, &sectors()[0..1]));
    assert_eq!(removed.faulty_power, power_for_sectors(SECTOR_SIZE, &sectors()[3..8]));
    assert_eq!(
        recovering_power,
        power_for_sectors(SECTOR_SIZE, &sectors()[5..6])
            + power_for_sectors(SECTOR_SIZE, &sectors()[7..8])
    );

    // assert queue state is as expected

    // only faulty sector 2 is found in first set
    require_no_expiration_groups_before(5, &mut queue);
    let set = queue.pop_until(5).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([2]));
    assert!(set.faulty_sectors.is_empty());
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([]));
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(1001));
    assert_eq!(set.active_power, PowerPair::zero());
    assert_eq!(set.faulty_power, power_for_sectors(SECTOR_SIZE, &sectors()[1..2]));

    // only faulty on-time sector 3 is found in second set
    require_no_expiration_groups_before(9, &mut queue);
    let set = queue.pop_until(9).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([3]));
    assert_eq!(set.proof_expiring_sectors, mk_bitfield([]));
    assert!(set.faulty_sectors.is_empty());
    assert_eq!(set.on_time_pledge, TokenAmount::from_atto(1002));
    assert_eq!(set.active_power, PowerPair::zero());
    assert_eq!(set.faulty_power, power_for_sectors(SECTOR_SIZE, &sectors()[2..3]));

    // no further sets remain
    require_no_expiration_groups_before(20, &mut queue);
}

#[test]
fn adding_no_sectors_leaves_the_queue_empty() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();

    let mut queue = empty_expiration_queue_with_quantizing(&rt, QuantSpec { unit: 4, offset: 1 });
    let _ = queue.add_active_sectors(&[SectorOnChainInfo::default(); 0], SECTOR_SIZE).unwrap();

    assert_eq!(queue.amt.count(), 0);
}

#[test]
fn rescheduling_no_expirations_as_faults_leaves_the_queue_empty() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();

    let mut queue = empty_expiration_queue_with_quantizing(&rt, QuantSpec { unit: 4, offset: 1 });
    let _ = queue.add_active_sectors(&sectors(), SECTOR_SIZE).unwrap();

    // all sectors already expire before epoch 15, nothing should change.
    let length = queue.amt.count();
    let _ = queue.reschedule_as_faults(15, &sectors(), SECTOR_SIZE).unwrap();
    assert_eq!(queue.amt.count(), length);
}

#[test]
fn rescheduling_all_expirations_as_faults_leaves_the_queue_empty_if_it_was_empty() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();

    let mut queue = empty_expiration_queue_with_quantizing(&rt, QuantSpec { unit: 4, offset: 1 });
    let _ = queue.add_active_sectors(&sectors(), SECTOR_SIZE).unwrap();

    // all sectors already expire before epoch 15, nothing should change.
    let length = queue.amt.count();
    queue.reschedule_all_as_faults(15).unwrap();
    assert_eq!(queue.amt.count(), length);
}

#[test]
fn rescheduling_no_sectors_as_recovered_leaves_the_queue_empty() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();

    let mut queue = empty_expiration_queue_with_quantizing(&rt, QuantSpec { unit: 4, offset: 1 });
    let _ = queue.reschedule_recovered(&[], SECTOR_SIZE).unwrap();
    assert!(queue.amt.count().is_zero());
}

fn test_sector(
    commitment_expiration: ChainEpoch,
    proof_expiration: ChainEpoch,
    sector_number: SectorNumber,
    weight: u64,
    vweight: u64,
    pledge: u64,
) -> SectorOnChainInfo {
    SectorOnChainInfo {
        commitment_expiration,
        proof_expiration,
        sector_number,
        deal_weight: DealWeight::from(weight),
        verified_deal_weight: DealWeight::from(vweight),
        initial_pledge: TokenAmount::from_atto(pledge),
        sealed_cid: make_sealed_cid(format!("commR-{}", sector_number).as_bytes()),
        ..Default::default()
    }
}

fn empty_expiration_queue_with_quantizing(
    rt: &MockRuntime,
    quant: QuantSpec,
) -> ExpirationQueue<MemoryBlockstore> {
    let empty_array =
        Amt::<(), _>::new_with_bit_width(&rt.store, TEST_AMT_BITWIDTH).flush().unwrap();

    ExpirationQueue::new(&*rt.store, &empty_array, quant).unwrap()
}

fn empty_expiration_queue(rt: &MockRuntime) -> ExpirationQueue<MemoryBlockstore> {
    empty_expiration_queue_with_quantizing(rt, NO_QUANTIZATION)
}
