use fil_actor_miner::{
    power_for_sectors, ExpirationQueue, ExpirationSet, PowerPair, SectorOnChainInfo,
};
use fil_actors_runtime::test_utils::MockRuntime;
use fvm_ipld_bitfield::{BitField, MaybeBitField};
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::{QuantSpec, NO_QUANTIZATION};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{SectorNumber, StoragePower};
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

use fil_actors_runtime::test_utils::make_sealed_cid;
use fil_actors_runtime::DealWeight;
use fvm_ipld_amt::Amt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::sector::SectorSize;

mod util;
use util::*;
// let empty_states_array =
//     Amt::<(), _>::new_with_bit_width(store, STATES_AMT_BITWIDTH).flush().unwrap();

// new(&rt.store, &empty_states_array, NO_QUANTIZATION)

const TEST_AMT_BITWIDTH: u32 = 3;

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

// #[test]
// fn quantizes_added_sectors_by_expiration
/*
    t.Run("quantizes added sectors by expiration", func(t *testing.T) {
        queue := emptyExpirationQueueWithQuantizing(t, builtin.NewQuantSpec(5, 3), testAmtBitwidth)
        secNums, power, pledge, err := queue.AddActiveSectors(sectors, sectorSize)
        require.NoError(t, err)
        assertBitfieldEquals(t, secNums, 1, 2, 3, 4, 5, 6)
        assert.True(t, power.Equals(miner.PowerForSectors(sectorSize, sectors)))
        assert.Equal(t, abi.NewTokenAmount(6015), pledge)

        // work around caching issues in amt
        _, err = queue.Root()
        require.NoError(t, err)

        // quantizing spec means sectors should be grouped into 3 sets expiring at 3, 8 and 13
        assert.Equal(t, 3, int(queue.Length()))

        // set popped before first quantized sector should be empty
        set, err := queue.PopUntil(2)
        require.NoError(t, err)
        assertBitfieldEmpty(t, set.OnTimeSectors)
        assert.Equal(t, 3, int(queue.Length()))

        // first 2 sectors will be in first set popped off at quantization offset (3)
        set, err = queue.PopUntil(3)
        require.NoError(t, err)
        assertBitfieldEquals(t, set.OnTimeSectors, 1, 2)
        assert.Equal(t, 2, int(queue.Length()))

        _, err = queue.Root()
        require.NoError(t, err)

        // no sectors will be popped off in quantization interval
        set, err = queue.PopUntil(7)
        require.NoError(t, err)
        assertBitfieldEmpty(t, set.OnTimeSectors)
        assert.Equal(t, 2, int(queue.Length()))

        // next 2 sectors will be in first set popped off after quantization interval (8)
        set, err = queue.PopUntil(8)
        require.NoError(t, err)
        assertBitfieldEquals(t, set.OnTimeSectors, 3, 4)
        assert.Equal(t, 1, int(queue.Length()))

        _, err = queue.Root()
        require.NoError(t, err)

        // no sectors will be popped off in quantization interval
        set, err = queue.PopUntil(12)
        require.NoError(t, err)
        assertBitfieldEmpty(t, set.OnTimeSectors)
        assert.Equal(t, 1, int(queue.Length()))

        // rest of sectors will be in first set popped off after quantization interval (13)
        set, err = queue.PopUntil(13)
        require.NoError(t, err)
        assertBitfieldEquals(t, set.OnTimeSectors, 5, 6)
        assert.Equal(t, 0, int(queue.Length()))
    })

    t.Run("reschedules sectors as faults", func(t *testing.T) {
        // Create 3 expiration sets with 2 sectors apiece
        queue := emptyExpirationQueueWithQuantizing(t, builtin.NewQuantSpec(4, 1), testAmtBitwidth)
        _, _, _, err := queue.AddActiveSectors(sectors, sectorSize)
        require.NoError(t, err)

        _, err = queue.Root()
        require.NoError(t, err)

        // Fault middle sectors to expire at epoch 6
        // This faults one sector from the first set, all of the second set and one from the third.
        // Faulting at epoch 6 means the first 3 will expire on time, but the last will be early and
        // moved to the second set
        powerDelta, err := queue.RescheduleAsFaults(abi.ChainEpoch(6), sectors[1:5], sectorSize)
        require.NoError(t, err)
        assert.True(t, powerDelta.Equals(miner.PowerForSectors(sectorSize, sectors[1:5])))

        _, err = queue.Root()
        require.NoError(t, err)

        // expect first set to contain first two sectors but with the seconds power moved to faulty power
        requireNoExpirationGroupsBefore(t, 5, queue)
        set, err := queue.PopUntil(5)
        require.NoError(t, err)

        assertBitfieldEquals(t, set.OnTimeSectors, 1, 2)
        assertBitfieldEmpty(t, set.EarlySectors)

        assert.Equal(t, big.NewInt(2001), set.OnTimePledge)
        assert.True(t, set.ActivePower.Equals(miner.PowerForSectors(sectorSize, sectors[0:1])))
        assert.True(t, set.FaultyPower.Equals(miner.PowerForSectors(sectorSize, sectors[1:2])))

        // expect the second set to have all faulty power and now contain 5th sector as an early sector
        requireNoExpirationGroupsBefore(t, 9, queue)
        set, err = queue.PopUntil(9)
        require.NoError(t, err)

        assertBitfieldEquals(t, set.OnTimeSectors, 3, 4)
        assertBitfieldEquals(t, set.EarlySectors, 5)

        // pledge is kept from original 2 sectors. Pledge from new early sector is NOT added.
        assert.Equal(t, big.NewInt(2005), set.OnTimePledge)

        assert.True(t, set.ActivePower.Equals(miner.NewPowerPairZero()))
        assert.True(t, set.FaultyPower.Equals(miner.PowerForSectors(sectorSize, sectors[2:5])))

        // expect last set to only contain non faulty sector
        requireNoExpirationGroupsBefore(t, 13, queue)
        set, err = queue.PopUntil(13)
        require.NoError(t, err)

        assertBitfieldEquals(t, set.OnTimeSectors, 6)
        assertBitfieldEmpty(t, set.EarlySectors)

        // Pledge from sector moved from this set is dropped
        assert.Equal(t, big.NewInt(1005), set.OnTimePledge)

        assert.True(t, set.ActivePower.Equals(miner.PowerForSectors(sectorSize, sectors[5:])))
        assert.True(t, set.FaultyPower.Equals(miner.NewPowerPairZero()))
    })

    t.Run("reschedules all sectors as faults", func(t *testing.T) {
        // Create expiration 3 sets with 2 sectors apiece
        queue := emptyExpirationQueueWithQuantizing(t, builtin.NewQuantSpec(4, 1), testAmtBitwidth)
        _, _, _, err := queue.AddActiveSectors(sectors, sectorSize)
        require.NoError(t, err)

        _, err = queue.Root()
        require.NoError(t, err)

        // Fault all sectors
        // This converts the first 2 sets to faults and adds the 3rd set as early sectors to the second set
        err = queue.RescheduleAllAsFaults(abi.ChainEpoch(6))
        require.NoError(t, err)

        _, err = queue.Root()
        require.NoError(t, err)

        // expect first set to contain first two sectors but with the seconds power moved to faulty power
        requireNoExpirationGroupsBefore(t, 5, queue)
        set, err := queue.PopUntil(5)
        require.NoError(t, err)

        assertBitfieldEquals(t, set.OnTimeSectors, 1, 2) // sectors are unmoved
        assertBitfieldEmpty(t, set.EarlySectors)

        assert.Equal(t, big.NewInt(2001), set.OnTimePledge) // pledge is same

        // active power is converted to fault power
        assert.True(t, set.ActivePower.Equals(miner.NewPowerPairZero()))
        assert.True(t, set.FaultyPower.Equals(miner.PowerForSectors(sectorSize, sectors[:2])))

        // expect the second set to have all faulty power and now contain 5th and 6th sectors as an early sectors
        requireNoExpirationGroupsBefore(t, 9, queue)
        set, err = queue.PopUntil(9)
        require.NoError(t, err)

        assertBitfieldEquals(t, set.OnTimeSectors, 3, 4)
        assertBitfieldEquals(t, set.EarlySectors, 5, 6)

        // pledge is kept from original 2 sectors. Pledge from new early sectors is NOT added.
        assert.Equal(t, big.NewInt(2005), set.OnTimePledge)

        // fault power is all power for sectors previously in the first and second sets
        assert.True(t, set.ActivePower.Equals(miner.NewPowerPairZero()))
        assert.True(t, set.FaultyPower.Equals(miner.PowerForSectors(sectorSize, sectors[2:])))

        // expect last set to only contain non faulty sector
        requireNoExpirationGroupsBefore(t, 13, queue)
        set, err = queue.PopUntil(13)
        require.NoError(t, err)

        assertBitfieldEmpty(t, set.OnTimeSectors)
        assertBitfieldEmpty(t, set.EarlySectors)

        // all pledge is dropped
        assert.Equal(t, big.Zero(), set.OnTimePledge)

        assert.True(t, set.ActivePower.Equals(miner.NewPowerPairZero()))
        assert.True(t, set.FaultyPower.Equals(miner.NewPowerPairZero()))
    })

    t.Run("reschedule recover restores all sector stats", func(t *testing.T) {
        // Create expiration 3 sets with 2 sectors apiece
        queue := emptyExpirationQueueWithQuantizing(t, builtin.NewQuantSpec(4, 1), testAmtBitwidth)
        _, _, _, err := queue.AddActiveSectors(sectors, sectorSize)
        require.NoError(t, err)

        _, err = queue.Root()
        require.NoError(t, err)

        // Fault middle sectors to expire at epoch 6 to put sectors in a state
        // described in "reschedules sectors as faults"
        _, err = queue.RescheduleAsFaults(abi.ChainEpoch(6), sectors[1:5], sectorSize)
        require.NoError(t, err)

        _, err = queue.Root()
        require.NoError(t, err)

        // mark faulted sectors as recovered
        recovered, err := queue.RescheduleRecovered(sectors[1:5], sectorSize)
        require.NoError(t, err)
        assert.True(t, recovered.Equals(miner.PowerForSectors(sectorSize, sectors[1:5])))

        // expect first set to contain first two sectors with active power
        requireNoExpirationGroupsBefore(t, 5, queue)
        set, err := queue.PopUntil(5)
        require.NoError(t, err)

        assertBitfieldEquals(t, set.OnTimeSectors, 1, 2)
        assertBitfieldEmpty(t, set.EarlySectors)

        // pledge from both sectors
        assert.Equal(t, big.NewInt(2001), set.OnTimePledge)

        assert.True(t, set.ActivePower.Equals(miner.PowerForSectors(sectorSize, sectors[:2])))
        assert.True(t, set.FaultyPower.Equals(miner.NewPowerPairZero()))

        // expect second set to have lost early sector 5 and have active power just from 3 and 4
        requireNoExpirationGroupsBefore(t, 9, queue)
        set, err = queue.PopUntil(9)
        require.NoError(t, err)

        assertBitfieldEquals(t, set.OnTimeSectors, 3, 4)
        assertBitfieldEmpty(t, set.EarlySectors)

        // pledge is kept from original 2 sectors
        assert.Equal(t, big.NewInt(2005), set.OnTimePledge)

        assert.True(t, set.ActivePower.Equals(miner.PowerForSectors(sectorSize, sectors[2:4])))
        assert.True(t, set.FaultyPower.Equals(miner.NewPowerPairZero()))

        // expect sector 5 to be returned to last setu
        requireNoExpirationGroupsBefore(t, 13, queue)
        set, err = queue.PopUntil(13)
        require.NoError(t, err)

        assertBitfieldEquals(t, set.OnTimeSectors, 5, 6)
        assertBitfieldEmpty(t, set.EarlySectors)

        // Pledge from sector 5 is restored
        assert.Equal(t, big.NewInt(2009), set.OnTimePledge)

        assert.True(t, set.ActivePower.Equals(miner.PowerForSectors(sectorSize, sectors[4:])))
        assert.True(t, set.FaultyPower.Equals(miner.NewPowerPairZero()))
    })

    t.Run("replaces sectors with new sectors", func(t *testing.T) {
        // Create expiration 3 sets
        queue := emptyExpirationQueueWithQuantizing(t, builtin.NewQuantSpec(4, 1), testAmtBitwidth)

        // add sectors to each set
        _, _, _, err := queue.AddActiveSectors([]*miner.SectorOnChainInfo{sectors[0], sectors[1], sectors[3], sectors[5]}, sectorSize)
        require.NoError(t, err)

        _, err = queue.Root()
        require.NoError(t, err)

        // remove all from first set, replace second set, and append to third
        toRemove := []*miner.SectorOnChainInfo{sectors[0], sectors[1], sectors[3]}
        toAdd := []*miner.SectorOnChainInfo{sectors[2], sectors[4]}
        removed, added, powerDelta, pledgeDelta, err := queue.ReplaceSectors(
            toRemove,
            toAdd,
            sectorSize)
        require.NoError(t, err)
        assertBitfieldEquals(t, removed, 1, 2, 4)
        assertBitfieldEquals(t, added, 3, 5)
        addedPower := miner.PowerForSectors(sectorSize, toAdd)
        assert.True(t, powerDelta.Equals(addedPower.Sub(miner.PowerForSectors(sectorSize, toRemove))))
        assert.Equal(t, abi.NewTokenAmount(1002+1004-1000-1001-1003), pledgeDelta)

        // first set is gone
        requireNoExpirationGroupsBefore(t, 9, queue)

        // second set is replaced
        set, err := queue.PopUntil(9)
        require.NoError(t, err)

        assertBitfieldEquals(t, set.OnTimeSectors, 3)
        assertBitfieldEmpty(t, set.EarlySectors)

        // pledge and power is only from sector 3
        assert.Equal(t, big.NewInt(1002), set.OnTimePledge)
        assert.True(t, set.ActivePower.Equals(miner.PowerForSectors(sectorSize, sectors[2:3])))
        assert.True(t, set.FaultyPower.Equals(miner.NewPowerPairZero()))

        // last set appends sector 6
        requireNoExpirationGroupsBefore(t, 13, queue)
        set, err = queue.PopUntil(13)
        require.NoError(t, err)

        assertBitfieldEquals(t, set.OnTimeSectors, 5, 6)
        assertBitfieldEmpty(t, set.EarlySectors)

        // pledge and power are some of old and new sectors
        assert.Equal(t, big.NewInt(2009), set.OnTimePledge)
        assert.True(t, set.ActivePower.Equals(miner.PowerForSectors(sectorSize, sectors[4:])))
        assert.True(t, set.FaultyPower.Equals(miner.NewPowerPairZero()))
    })

    t.Run("removes sectors", func(t *testing.T) {
        // add all sectors into 3 sets
        queue := emptyExpirationQueueWithQuantizing(t, builtin.NewQuantSpec(4, 1), testAmtBitwidth)
        _, _, _, err := queue.AddActiveSectors(sectors, sectorSize)
        require.NoError(t, err)

        _, err = queue.Root()
        require.NoError(t, err)

        // put queue in a state where some sectors are early and some are faulty
        _, err = queue.RescheduleAsFaults(abi.ChainEpoch(6), sectors[1:6], sectorSize)
        require.NoError(t, err)

        _, err = queue.Root()
        require.NoError(t, err)

        // remove an active sector from first set, faulty sector and early faulty sector from second set,
        toRemove := []*miner.SectorOnChainInfo{sectors[0], sectors[3], sectors[4], sectors[5]}

        // and only sector from last set
        faults := bitfield.NewFromSet([]uint64{4, 5, 6})

        // label the last as recovering
        recovering := bitfield.NewFromSet([]uint64{6})
        removed, recoveringPower, err := queue.RemoveSectors(toRemove, faults, recovering, sectorSize)
        require.NoError(t, err)

        // assert all return values are correct
        assertBitfieldEquals(t, removed.OnTimeSectors, 1, 4)
        assertBitfieldEquals(t, removed.EarlySectors, 5, 6)
        assert.Equal(t, abi.NewTokenAmount(1000+1003), removed.OnTimePledge) // only on-time sectors
        assert.True(t, removed.ActivePower.Equals(miner.PowerForSectors(sectorSize, []*miner.SectorOnChainInfo{sectors[0]})))
        assert.True(t, removed.FaultyPower.Equals(miner.PowerForSectors(sectorSize, sectors[3:6])))
        assert.True(t, recoveringPower.Equals(miner.PowerForSectors(sectorSize, sectors[5:6])))

        // assert queue state is as expected

        // only faulty sector 2 is found in first set
        requireNoExpirationGroupsBefore(t, 5, queue)
        set, err := queue.PopUntil(5)
        require.NoError(t, err)

        assertBitfieldEquals(t, set.OnTimeSectors, 2)
        assertBitfieldEmpty(t, set.EarlySectors)
        assert.Equal(t, abi.NewTokenAmount(1001), set.OnTimePledge)
        assert.True(t, set.ActivePower.Equals(miner.NewPowerPairZero()))
        assert.True(t, set.FaultyPower.Equals(miner.PowerForSectors(sectorSize, sectors[1:2])))

        // only faulty on-time sector 3 is found in second set
        requireNoExpirationGroupsBefore(t, 9, queue)
        set, err = queue.PopUntil(9)
        require.NoError(t, err)

        assertBitfieldEquals(t, set.OnTimeSectors, 3)
        assertBitfieldEmpty(t, set.EarlySectors)
        assert.Equal(t, abi.NewTokenAmount(1002), set.OnTimePledge)
        assert.True(t, set.ActivePower.Equals(miner.NewPowerPairZero()))
        assert.True(t, set.FaultyPower.Equals(miner.PowerForSectors(sectorSize, sectors[2:3])))

        // no further sets remain
        requireNoExpirationGroupsBefore(t, 20, queue)
    })

    t.Run("adding no sectors leaves the queue empty", func(t *testing.T) {
        queue := emptyExpirationQueueWithQuantizing(t, builtin.NewQuantSpec(4, 1), testAmtBitwidth)
        _, _, _, err := queue.AddActiveSectors(nil, sectorSize)
        require.NoError(t, err)
        assert.Zero(t, queue.Length())
    })

    t.Run("rescheduling no expirations as faults leaves the queue empty", func(t *testing.T) {
        queue := emptyExpirationQueueWithQuantizing(t, builtin.NewQuantSpec(4, 1), testAmtBitwidth)

        _, _, _, err := queue.AddActiveSectors(sectors, sectorSize)
        require.NoError(t, err)

        // all sectors already expire before epoch 15, nothing should change.
        length := queue.Length()
        _, err = queue.RescheduleAsFaults(15, sectors, sectorSize)
        require.NoError(t, err)
        assert.Equal(t, length, queue.Length())
    })

    t.Run("rescheduling all expirations as faults leaves the queue empty if it was empty", func(t *testing.T) {
        queue := emptyExpirationQueueWithQuantizing(t, builtin.NewQuantSpec(4, 1), testAmtBitwidth)

        _, _, _, err := queue.AddActiveSectors(sectors, sectorSize)
        require.NoError(t, err)

        // all sectors already expire before epoch 15, nothing should change.
        length := queue.Length()
        err = queue.RescheduleAllAsFaults(15)
        require.NoError(t, err)
        assert.Equal(t, length, queue.Length())
    })

    t.Run("rescheduling no sectors as recovered leaves the queue empty", func(t *testing.T) {
        queue := emptyExpirationQueueWithQuantizing(t, builtin.NewQuantSpec(4, 1), testAmtBitwidth)
        _, err := queue.RescheduleRecovered(nil, sectorSize)
        require.NoError(t, err)
        assert.Zero(t, queue.Length())
    })
}

func testSector(expiration, number, weight, vweight, pledge int64) *miner.SectorOnChainInfo {
    return &miner.SectorOnChainInfo{
        Expiration:         abi.ChainEpoch(expiration),
        SectorNumber:       abi.SectorNumber(number),
        DealWeight:         big.NewInt(weight),
        VerifiedDealWeight: big.NewInt(vweight),
        InitialPledge:      abi.NewTokenAmount(pledge),
        SealedCID:          tutil.MakeCID(fmt.Sprintf("commR-%d", number), &miner.SealedCIDPrefix),
    }
}

func requireNoExpirationGroupsBefore(t *testing.T, epoch abi.ChainEpoch, queue miner.ExpirationQueue) {
    _, err := queue.Root()
    require.NoError(t, err)

    set, err := queue.PopUntil(epoch - 1)
    require.NoError(t, err)
    empty, err := set.IsEmpty()
    require.NoError(t, err)
    require.True(t, empty)
}

func emptyExpirationQueueWithQuantizing(t *testing.T, quant builtin.QuantSpec, bitwidth int) miner.ExpirationQueue {
    rt := mock.NewBuilder(address.Undef).Build(t)
    store := adt.AsStore(rt)
    emptyArray, err := adt.StoreEmptyArray(store, testAmtBitwidth)
    require.NoError(t, err)

    queue, err := miner.LoadExpirationQueue(store, emptyArray, quant, bitwidth)
    require.NoError(t, err)
    return queue
}

func emptyExpirationQueue(t *testing.T) miner.ExpirationQueue {
    return emptyExpirationQueueWithQuantizing(t, builtin.NoQuantization, testAmtBitwidth)
}
 */

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
