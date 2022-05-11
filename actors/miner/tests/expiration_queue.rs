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
    //      See: https://github.com/filecoin-project/builtin-actors/issues/338
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

fn sectors() -> [SectorOnChainInfo; 6] {
    [
        test_sector(2, 1, 50, 60, 1000),
        test_sector(3, 2, 51, 61, 1001),
        test_sector(7, 3, 52, 62, 1002),
        test_sector(8, 4, 53, 63, 1003),
        test_sector(11, 5, 54, 64, 1004),
        test_sector(13, 6, 55, 65, 1005),
    ]
}

const SECTOR_SIZE: SectorSize = SectorSize::_32GiB;

#[test]
fn added_sectors_can_be_popped_off_queue() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();

    let mut queue = empty_expiration_queue(&rt);
    let (sec_nums, power, pledge) = queue.add_active_sectors(&sectors(), SECTOR_SIZE).unwrap();
    assert_eq!(sec_nums, mk_bitfield([1, 2, 3, 4, 5, 6]));
    assert_eq!(power, power_for_sectors(SECTOR_SIZE, &sectors()));
    assert_eq!(pledge, TokenAmount::from(6015));

    // default test quantizing of 1 means every sector is in its own expriation set
    assert_eq!(sectors().len(), queue.amt.count() as usize);

    queue.amt.flush().unwrap();

    // pop off sectors up to and including epoch 8
    let set = queue.pop_until(7).unwrap();

    // only 3 sectors remain
    assert_eq!(queue.amt.count(), 3);

    assert_eq!(set.on_time_sectors, mk_bitfield([1, 2, 3]));
    assert!(set.early_sectors.is_empty());

    let active_power = power_for_sectors(SECTOR_SIZE, &sectors()[0..3]);
    let faulty_power = PowerPair::zero();

    assert_eq!(set.on_time_pledge, TokenAmount::from(3003));
    assert_eq!(set.active_power, active_power);
    assert_eq!(set.faulty_power, faulty_power);

    // pop off rest up to and including epoch 8
    let set = queue.pop_until(20).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([4, 5, 6]));
    assert!(set.early_sectors.is_empty());

    assert_eq!(set.on_time_pledge, TokenAmount::from(3012)); // sum of last 3 sector pledges
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
    assert_eq!(sec_nums, mk_bitfield([1, 2, 3, 4, 5, 6]));
    assert_eq!(power, power_for_sectors(SECTOR_SIZE, &sectors()));
    assert_eq!(pledge, TokenAmount::from(6015));

    // quantizing spec means sectors should be grouped into 3 sets expiring at 3, 8 and 13
    assert_eq!(queue.amt.count(), 3);

    // set popped before first quantized sector should be empty
    let set = queue.pop_until(2).unwrap();
    assert!(set.on_time_sectors.is_empty());
    assert_eq!(queue.amt.count(), 3);

    // first 2 sectors will be in first set popped off at quantization offset (3)
    let set = queue.pop_until(3).unwrap();
    assert_eq!(set.on_time_sectors, mk_bitfield([1, 2]));
    assert_eq!(queue.amt.count(), 2);

    let _ = queue.amt.flush().unwrap();

    // no sectors will be popped off in quantization interval
    let set = queue.pop_until(7).unwrap();
    assert!(set.on_time_sectors.is_empty());
    assert_eq!(queue.amt.count(), 2);

    // next 2 sectors will be in first set popped off after quantization interval (8)
    let set = queue.pop_until(8).unwrap();
    assert_eq!(set.on_time_sectors, mk_bitfield([3, 4]));
    assert_eq!(queue.amt.count(), 1);

    let _ = queue.amt.flush().unwrap();

    // no sectors will be popped off in quantization interval
    let set = queue.pop_until(12).unwrap();
    assert!(set.on_time_sectors.is_empty());
    assert_eq!(queue.amt.count(), 1);

    // rest of sectors will be in first set popped off after quantization interval (13)
    let set = queue.pop_until(13).unwrap();
    assert_eq!(set.on_time_sectors, mk_bitfield([5, 6]));
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
    // Faulting at epoch 6 means the first 3 will expire on time, but the last will be early and
    // moved to the second set
    let power_delta = queue.reschedule_as_faults(6, &sectors()[1..5], SECTOR_SIZE).unwrap();
    assert_eq!(power_delta, power_for_sectors(SECTOR_SIZE, &sectors()[1..5]));

    let _ = queue.amt.flush().unwrap();

    // expect first set to contain first two sectors but with the seconds power moved to faulty power
    require_no_expiration_groups_before(5, &mut queue);
    let set = queue.pop_until(5).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([1, 2]));
    assert!(set.early_sectors.is_empty());

    assert_eq!(set.on_time_pledge, TokenAmount::from(2001));
    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors()[0..1]));
    assert_eq!(set.faulty_power, power_for_sectors(SECTOR_SIZE, &sectors()[1..2]));

    // expect the second set to have all faulty power and now contain 5th sector as an early sector
    require_no_expiration_groups_before(9, &mut queue);
    let set = queue.pop_until(9).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([3, 4]));
    assert_eq!(set.early_sectors, mk_bitfield([5]));

    // pledge is kept from original 2 sectors. Pledge from new early sector is NOT added.
    assert_eq!(set.on_time_pledge, TokenAmount::from(2005));

    assert_eq!(set.active_power, PowerPair::zero());
    assert_eq!(set.faulty_power, power_for_sectors(SECTOR_SIZE, &sectors()[2..5]));

    // expect last set to only contain non faulty sector
    require_no_expiration_groups_before(13, &mut queue);
    let set = queue.pop_until(13).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([6]));
    assert!(set.early_sectors.is_empty());

    // Pledge from sector moved from this set is dropped
    assert_eq!(set.on_time_pledge, TokenAmount::from(1005));

    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors()[5..]));
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

    // Fault all sectors
    // This converts the first 2 sets to faults and adds the 3rd set as early sectors to the second set
    queue.reschedule_all_as_faults(6).unwrap();

    let _ = queue.amt.flush().unwrap();

    // expect first set to contain first two sectors but with the seconds power moved to faulty power
    require_no_expiration_groups_before(5, &mut queue);
    let set = queue.pop_until(5).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([1, 2])); // sectors are unmoved
    assert!(set.early_sectors.is_empty());

    assert_eq!(set.on_time_pledge, TokenAmount::from(2001)); // pledge is same

    // active power is converted to fault power
    assert_eq!(set.active_power, PowerPair::zero());
    assert_eq!(set.faulty_power, power_for_sectors(SECTOR_SIZE, &sectors()[0..2]));

    // expect the second set to have all faulty power and now contain 5th and 6th sectors as an early sectors
    require_no_expiration_groups_before(9, &mut queue);
    let set = queue.pop_until(9).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([3, 4]));
    assert_eq!(set.early_sectors, mk_bitfield([5, 6]));

    // pledge is kept from original 2 sectors. Pledge from new early sectors is NOT added.
    assert_eq!(set.on_time_pledge, TokenAmount::from(2005));

    // fault power is all power for sectors previously in the first and second sets
    assert_eq!(set.active_power, PowerPair::zero());
    assert_eq!(set.faulty_power, power_for_sectors(SECTOR_SIZE, &sectors()[2..]));

    // expect last set to only contain non faulty sector
    require_no_expiration_groups_before(13, &mut queue);
    let set = queue.pop_until(13).unwrap();

    assert!(set.on_time_sectors.is_empty());
    assert!(set.early_sectors.is_empty());

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
    let _ = queue.reschedule_as_faults(6, &sectors()[1..5], SECTOR_SIZE).unwrap();

    let _ = queue.amt.flush().unwrap();

    // mark faulted sectors as recovered
    let recovered = queue.reschedule_recovered(sectors()[1..5].to_vec(), SECTOR_SIZE).unwrap();
    assert_eq!(recovered, power_for_sectors(SECTOR_SIZE, &sectors()[1..5]));

    // expect first set to contain first two sectors with active power
    require_no_expiration_groups_before(5, &mut queue);
    let set = queue.pop_until(5).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([1, 2]));
    assert!(set.early_sectors.is_empty());

    // pledge from both sectors
    assert_eq!(set.on_time_pledge, TokenAmount::from(2001));

    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors()[0..2]));
    assert_eq!(set.faulty_power, PowerPair::zero());

    // expect second set to have lost early sector 5 and have active power just from 3 and 4
    require_no_expiration_groups_before(9, &mut queue);
    let set = queue.pop_until(9).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([3, 4]));
    assert!(set.early_sectors.is_empty());

    // pledge is kept from original 2 sectors
    assert_eq!(set.on_time_pledge, TokenAmount::from(2005));

    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors()[2..4]));
    assert_eq!(set.faulty_power, PowerPair::zero());

    // expect sector 5 to be returned to last setu
    require_no_expiration_groups_before(13, &mut queue);
    let set = queue.pop_until(13).unwrap();

    assert_eq!(set.on_time_sectors, mk_bitfield([5, 6]));
    assert!(set.early_sectors.is_empty());

    // Pledge from sector 5 is restored
    assert_eq!(set.on_time_pledge, TokenAmount::from(2009));

    assert_eq!(set.active_power, power_for_sectors(SECTOR_SIZE, &sectors()[4..]));
    assert_eq!(set.faulty_power, PowerPair::zero());
}

fn test_sector(
    expiration: ChainEpoch,
    sector_number: SectorNumber,
    weight: u64,
    vweight: u64,
    pledge: u64,
) -> SectorOnChainInfo {
    SectorOnChainInfo {
        expiration,
        sector_number,
        deal_weight: DealWeight::from(weight),
        verified_deal_weight: DealWeight::from(vweight),
        initial_pledge: TokenAmount::from(pledge),
        sealed_cid: make_sealed_cid(format!("commR-{}", sector_number).as_bytes()),
        ..Default::default()
    }
}

fn require_no_expiration_groups_before(
    epoch: ChainEpoch,
    queue: &mut ExpirationQueue<'_, MemoryBlockstore>,
) {
    queue.amt.flush().unwrap();

    let set = queue.pop_until(epoch - 1).unwrap();
    assert!(set.is_empty());
}

fn empty_expiration_queue_with_quantizing<'a>(
    rt: &'a MockRuntime,
    quant: QuantSpec,
) -> ExpirationQueue<'a, MemoryBlockstore> {
    let empty_array =
        Amt::<(), _>::new_with_bit_width(&rt.store, TEST_AMT_BITWIDTH).flush().unwrap();

    ExpirationQueue::new(&rt.store, &empty_array, quant).unwrap()
}

fn empty_expiration_queue<'a>(rt: &'a MockRuntime) -> ExpirationQueue<'a, MemoryBlockstore> {
    empty_expiration_queue_with_quantizing(rt, NO_QUANTIZATION)
}
