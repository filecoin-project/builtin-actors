use std::collections::{BTreeSet, HashMap};

use fil_actor_miner::{
    power_for_sectors, Deadline, PartitionSectorMap, PoStPartition, PowerPair, SectorOnChainInfo,
    TerminationResult,
};
use fil_actors_runtime::runtime::{Policy, Runtime};
use fil_actors_runtime::test_utils::{MessageAccumulator, MockRuntime};
use fil_actors_runtime::ActorError;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_bitfield::UnvalidatedBitField;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::{clock::QuantSpec, error::ExitCode, sector::SectorSize};

mod util;
use crate::util::*;

fn sectors() -> Vec<SectorOnChainInfo> {
    vec![
        test_sector(2, 1, 50, 60, 1000),
        test_sector(3, 2, 51, 61, 1001),
        test_sector(7, 3, 52, 62, 1002),
        test_sector(8, 4, 53, 63, 1003),
        test_sector(8, 5, 54, 64, 1004),
        test_sector(11, 6, 55, 65, 1005),
        test_sector(13, 7, 56, 66, 1006),
        test_sector(8, 8, 57, 67, 1007),
        test_sector(8, 9, 58, 68, 1008),
    ]
}

fn extra_sectors() -> Vec<SectorOnChainInfo> {
    vec![test_sector(8, 10, 58, 68, 1008)]
}

fn all_sectors() -> Vec<SectorOnChainInfo> {
    [sectors(), extra_sectors()].concat()
}

const PARTITION_SIZE: u64 = 4;

// Define some basic test scenarios that build one each other.
//

// Adds sectors, and proves them if requested.
//
// Partition 1: sectors 1, 2, 3, 4
// Partition 2: sectors 5, 6, 7, 8
// Partition 3: sectors 9
fn add_sectors(
    rt: &MockRuntime,
    deadline: &mut Deadline,
    prove: bool,
) -> (ExpectedDeadlineState, Vec<SectorOnChainInfo>) {
    let sectors = sectors();
    let store = rt.store();

    let power = power_for_sectors(SECTOR_SIZE, &sectors);
    let activated_power = deadline
        .add_sectors(store, PARTITION_SIZE, false, &sectors, SECTOR_SIZE, QUANT_SPEC)
        .expect("Couldn't add sectors");

    assert_eq!(activated_power, power);

    let deadline_state = deadline_state()
        .with_unproven(&[1, 2, 3, 4, 5, 6, 7, 8, 9])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(store, &sectors, deadline);

    if !prove {
        return (deadline_state, sectors);
    }

    let mut sector_array = sectors_array(rt, store, sectors.to_owned());

    //prove everything
    let result = deadline
        .record_proven_sectors(
            store,
            &sector_array,
            SECTOR_SIZE,
            QUANT_SPEC,
            0,
            &mut [
                PoStPartition {
                    index: 0,
                    skipped: UnvalidatedBitField::Validated(BitField::default()),
                },
                PoStPartition {
                    index: 1,
                    skipped: UnvalidatedBitField::Validated(BitField::default()),
                },
                PoStPartition {
                    index: 2,
                    skipped: UnvalidatedBitField::Validated(BitField::default()),
                },
            ],
        )
        .unwrap();

    assert_eq!(result.power_delta, power);

    let sectors_root = sector_array.amt.flush().unwrap();

    let (faulty_power, recovery_power) =
        deadline.process_deadline_end(store, QUANT_SPEC, 0, sectors_root).unwrap();

    assert!(faulty_power.is_zero());
    assert!(recovery_power.is_zero());

    let deadline_state = deadline_state
        .with_unproven(&[])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(store, &sectors, deadline);

    (deadline_state, sectors)
}

// Adds sectors according to addSectors, then terminates them:
//
// From partition 0: sectors 1 & 3
// From partition 1: sectors 6
fn add_then_terminate(
    rt: &MockRuntime,
    deadline: &mut Deadline,
    prove: bool,
) -> (ExpectedDeadlineState, Vec<SectorOnChainInfo>) {
    let (deadline_state, sectors) = add_sectors(rt, deadline, prove);

    let removed_power = terminate_sectors(
        rt,
        deadline,
        15,
        sectors.to_owned(),
        HashMap::from([(0, new_bitfield(&[1, 3])), (1, new_bitfield(&[6]))]),
    )
    .unwrap();

    let (expected_power, unproven) = if prove {
        (sector_power(&[1, 3, 6]), vec![])
    } else {
        (PowerPair::zero(), vec![2, 4, 5, 7, 8, 9]) // not 1,3,6
    };

    assert_eq!(expected_power, removed_power);

    let deadline_state = deadline_state
        .with_terminations(&[1, 3, 6])
        .with_unproven(&unproven)
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(rt.store(), &sectors, deadline);

    (deadline_state, sectors)
}

// Adds and terminates sectors according to the previous two functions,
// then pops early terminations.
fn add_then_terminate_then_pop_early(
    rt: &MockRuntime,
    deadline: &mut Deadline,
) -> (ExpectedDeadlineState, Vec<SectorOnChainInfo>) {
    let (deadline_state, sectors) = add_then_terminate(rt, deadline, true);
    let store = rt.store();

    let (early_terminations, has_more) = deadline.pop_early_terminations(store, 100, 100).unwrap();

    assert!(!has_more);
    assert_eq!(2, early_terminations.partitions_processed);
    assert_eq!(3, early_terminations.sectors_processed);
    assert_eq!(1, early_terminations.sectors.len());

    assert_bitfield_equals(early_terminations.sectors.get(&15).unwrap(), &[1, 3, 6]);

    // Popping early terminations doesn't affect the terminations bitfield.
    let deadline_state = deadline_state
        .with_terminations(&[1, 3, 6])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(store, &sectors, deadline);

    (deadline_state, sectors)
}

// Runs the above scenarios, then removes partition 0.
fn add_then_terminate_then_remove_partition(
    rt: &MockRuntime,
    deadline: &mut Deadline,
) -> (ExpectedDeadlineState, Vec<SectorOnChainInfo>) {
    let (deadline_state, sectors) = add_then_terminate_then_pop_early(rt, deadline);
    let store = rt.store();

    let (live, dead, removed_power) = deadline
        .remove_partitions(store, &new_bitfield(&[0]), QUANT_SPEC)
        .expect("should have removed partitions");

    assert_bitfield_equals(&live, &[2, 4]);
    assert_bitfield_equals(&dead, &[1, 3]);

    let live_power = power_for_sectors(SECTOR_SIZE, &select_sectors(&sectors, &live));
    assert_eq!(live_power, removed_power);

    let deadline_state = deadline_state
        .with_terminations(&[6])
        .with_partitions(vec![new_bitfield(&[5, 6, 7, 8]), new_bitfield(&[9])])
        .assert(store, &sectors, deadline);

    (deadline_state, sectors)
}
// Adds sectors according to addSectors, then marks sectors 1, 5, 6
// faulty, expiring at epoch 9.
//
// Sector 5 will expire on-time at epoch 9 while 6 will expire early at epoch 9.
fn add_then_mark_faulty(
    rt: &MockRuntime,
    deadline: &mut Deadline,
    prove: bool,
) -> (ExpectedDeadlineState, Vec<SectorOnChainInfo>) {
    let (deadline_state, sectors) = add_sectors(rt, deadline, prove);

    let store = rt.store();
    let sectors_array = sectors_array(rt, store, sectors.to_owned());

    let mut partition_sector_map = PartitionSectorMap::default();
    partition_sector_map.add(0, UnvalidatedBitField::Validated(new_bitfield(&[1]))).unwrap();
    partition_sector_map.add(1, UnvalidatedBitField::Validated(new_bitfield(&[5, 6]))).unwrap();

    // mark faulty
    let power_delta = deadline
        .record_faults(store, &sectors_array, SECTOR_SIZE, QUANT_SPEC, 9, &mut partition_sector_map)
        .unwrap();

    let (expected_power, unproven) = if prove {
        (sector_power(&[1, 5, 6]), vec![])
    } else {
        (PowerPair::zero(), vec![2, 3, 4, 7, 8, 9]) // not 1,5,6
    };

    assert_eq!(power_delta, -expected_power);

    let deadline_state = deadline_state
        .with_faults(&[1, 5, 6])
        .with_unproven(&unproven)
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(store, &sectors, deadline);
    (deadline_state, sectors)
}

#[test]
fn adds_sectors() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_sectors(&rt, &mut deadline, false);
}

#[test]
fn adds_sectors_and_proves() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_sectors(&rt, &mut deadline, true);
}

#[test]
fn terminates_sectors() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_then_terminate(&rt, &mut deadline, true);
}

#[test]
fn terminates_unproven_sectors() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_then_terminate(&rt, &mut deadline, false);
}

#[test]
fn pops_early_terminations() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_then_terminate_then_pop_early(&rt, &mut deadline);
}

#[test]
fn removes_partitions() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_then_terminate_then_remove_partition(&rt, &mut deadline);
}

#[test]
fn marks_faulty() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_then_mark_faulty(&rt, &mut deadline, true);
}

#[test]
fn marks_unproven_sectors_faulty() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_then_mark_faulty(&rt, &mut deadline, false);
}

#[test]
fn cannot_remove_partitions_with_early_terminations() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_then_terminate(&rt, &mut deadline, false);

    let store = rt.store();
    assert!(deadline.remove_partitions(store, &new_bitfield(&[0]), QUANT_SPEC).is_err());
}

#[test]
fn can_pop_early_terminations_in_multiple_steps() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    let (deadline_state, sectors) = add_then_terminate(&rt, &mut deadline, true);

    let mut result = TerminationResult::new();

    let store = rt.store();

    // process 1 sector, 2 partitions (should pop 1 sector)
    let (partial, has_more) = deadline.pop_early_terminations(store, 2, 1).unwrap();
    assert!(has_more);
    result += partial;

    // process 2 sectors, 1 partition (should pop 1 sector)
    let (partial, has_more) = deadline.pop_early_terminations(store, 2, 1).unwrap();
    assert!(has_more);
    result += partial;

    // process 1 sector, 1 partition (should pop 1 sector)
    let (partial, has_more) = deadline.pop_early_terminations(store, 2, 1).unwrap();
    assert!(!has_more);
    result += partial;

    assert_eq!(3, result.partitions_processed);
    assert_eq!(3, result.sectors_processed);
    assert_eq!(1, result.sectors.len());
    assert_bitfield_equals(result.sectors.get(&15).unwrap(), &[1, 3, 6]);

    // Popping early terminations doesn't affect the terminations bitfield.
    deadline_state
        .with_terminations(&[1, 3, 6])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(store, &sectors, &deadline);
}

#[test]
fn cannot_remove_missing_partition() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_then_terminate_then_remove_partition(&rt, &mut deadline);
    assert!(deadline.remove_partitions(rt.store(), &new_bitfield(&[2]), QUANT_SPEC).is_err());
}

#[test]
fn removing_no_partitions_does_nothing() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    let (deadline_state, sectors) = add_then_terminate_then_pop_early(&rt, &mut deadline);
    let (live, dead, removed_power) = deadline
        .remove_partitions(rt.store(), &new_bitfield(&[]), QUANT_SPEC)
        .expect("should not have failed to remove partitions");

    assert!(removed_power.is_zero());
    assert!(live.is_empty());
    assert!(dead.is_empty());

    // Popping early terminations doesn't affect the terminations bitfield.
    deadline_state
        .with_terminations(&[1, 3, 6])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(rt.store(), &sectors, &deadline);
}

#[test]
fn fails_to_remove_partitions_with_faulty_sectors() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_then_mark_faulty(&rt, &mut deadline, false);

    // Try to remove a partition with faulty sectors.
    assert!(deadline.remove_partitions(rt.store(), &new_bitfield(&[1]), QUANT_SPEC).is_err());
}

#[test]
fn terminate_proven_and_faulty() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    let (deadline_state, sectors) = add_then_mark_faulty(&rt, &mut deadline, true); // 1,5,6 faulty

    let removed_power = terminate_sectors(
        &rt,
        &mut deadline,
        15,
        sectors.to_owned(),
        HashMap::from([(0, new_bitfield(&[1, 3])), (1, new_bitfield(&[6]))]),
    )
    .unwrap();

    // Sector 3 active, 1, 6 faulty
    let expected_power_loss =
        power_for_sectors(SECTOR_SIZE, &select_sectors(&sectors, &new_bitfield(&[3])));
    assert_eq!(
        expected_power_loss, removed_power,
        "deadline state to remove power for terminated sectors"
    );

    deadline_state
        .with_terminations(&[1, 3, 6])
        .with_faults(&[5])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(rt.store(), &sectors, &deadline);
}

fn terminate_sectors(
    rt: &MockRuntime,
    deadline: &mut Deadline,
    epoch: ChainEpoch,
    sectors: Vec<SectorOnChainInfo>,
    partition_sectors: HashMap<u64, BitField>,
) -> anyhow::Result<PowerPair> {
    let store = rt.store();
    let sectors_array = sectors_array(rt, &store, sectors);

    let mut partition_sector_map = PartitionSectorMap::default();
    for (partition, sectors) in partition_sectors {
        partition_sector_map.add(partition, UnvalidatedBitField::Validated(sectors)).unwrap();
    }

    deadline.terminate_sectors(
        &Policy::default(),
        &store,
        &sectors_array,
        epoch,
        &mut partition_sector_map,
        SECTOR_SIZE,
        QUANT_SPEC,
    )
}

#[test]
fn terminate_unproven_and_faulty() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    let (deadline_state, sectors) = add_then_mark_faulty(&rt, &mut deadline, false); // 1,5,6 faulty

    let removed_power = terminate_sectors(
        &rt,
        &mut deadline,
        15,
        sectors.to_owned(),
        HashMap::from([(0, new_bitfield(&[1, 3])), (1, new_bitfield(&[6]))]),
    )
    .unwrap();

    // sector 3 unproven, 1, 6 faulty
    assert!(removed_power.is_zero(), "should remove no power");

    deadline_state
        .with_terminations(&[1, 3, 6])
        .with_faults(&[5])
        .with_unproven(&[2, 4, 7, 8, 9]) // not 1, 3, 5, & 6
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(rt.store(), &sectors, &deadline);
}

#[test]
fn fails_to_terminate_missing_sector() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();
    let (_, sectors) = add_then_mark_faulty(&rt, &mut deadline, false); // 1,5,6 faulty

    let ret = terminate_sectors(
        &rt,
        &mut deadline,
        15,
        sectors,
        HashMap::from([(0, new_bitfield(&[6]))]),
    );

    assert!(ret.is_err());
    let err = ret
        .err()
        .expect("can only terminate live sectors")
        .downcast::<ActorError>()
        .expect("Invalid error");
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn fails_to_terminate_missing_partition() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();
    let (_, sectors) = add_then_mark_faulty(&rt, &mut deadline, false); // 1,5,6 faulty

    let ret = terminate_sectors(
        &rt,
        &mut deadline,
        15,
        sectors,
        HashMap::from([(4, new_bitfield(&[6]))]),
    );

    assert!(ret.is_err());
    let err = ret
        .err()
        .expect("can only terminate existing partitions")
        .downcast::<ActorError>()
        .expect("Invalid error");
    assert_eq!(err.exit_code(), ExitCode::USR_NOT_FOUND);
}

#[test]
fn fails_to_terminate_already_terminated_sector() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();
    let (_, sectors) = add_then_terminate(&rt, &mut deadline, false); // terminates 1,3,6

    let ret = terminate_sectors(
        &rt,
        &mut deadline,
        15,
        sectors,
        HashMap::from([(0, new_bitfield(&[1, 2]))]),
    );

    assert!(ret.is_err());
    let err = ret
        .err()
        .expect("cannot terminate already terminated sector")
        .downcast::<ActorError>()
        .expect("Invalid error");
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn faulty_sectors_expire() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    // mark sectors 5&6 faulty, expiring at epoch 9
    let (_, sectors) = add_then_mark_faulty(&rt, &mut deadline, true);

    // we expect all sectors but 7 to have expired at this point
    let expired = deadline
        .pop_expired_sectors(rt.store(), 9, QUANT_SPEC)
        .expect("failed to pop expired sectors");

    assert_bitfield_equals(&expired.on_time_sectors, &[1, 2, 3, 4, 5, 8, 9]);
    assert_bitfield_equals(&expired.early_sectors, &[6]);

    deadline_state()
        .with_terminations(&[1, 2, 3, 4, 5, 6, 8, 9])
        .with_faults(&[])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(rt.store(), &sectors, &deadline);

    // check early terminations
    let (early_terminations, has_more) = deadline
        .pop_early_terminations(rt.store(), 100, 100)
        .expect("failed to pop early_terminations");
    assert!(!has_more);
    assert_eq!(early_terminations.partitions_processed, 1);
    assert_eq!(early_terminations.sectors_processed, 1);
    assert_eq!(early_terminations.sectors.len(), 1);
    assert_bitfield_equals(early_terminations.sectors.get(&9).unwrap(), &[6]);

    // popping early_terminations doesn't affect the terminations bitfield
    deadline_state()
        .with_terminations(&[1, 2, 3, 4, 5, 6, 8, 9])
        .with_faults(&[])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(rt.store(), &sectors, &deadline);
}

#[test]
fn cannot_pop_expired_sectors_before_proving() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    // add sectors, but don't prove
    add_sectors(&rt, &mut deadline, false);

    // try to pop some expirations
    let ret = deadline.pop_expired_sectors(rt.store(), 9, QUANT_SPEC);
    assert!(ret.is_err());
    let err = ret.err().expect("cannot pop expired sectors from a partition with unproven sectors");

    assert!(err
        .to_string()
        .to_lowercase()
        .contains("cannot pop expired sectors from a partition with unproven sectors"));
}

#[test]
fn post_all_the_things() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();
    let fault_expiration_epoch = 13;

    add_sectors(&rt, &mut deadline, true);

    // add an inactive sector
    let power = deadline
        .add_sectors(rt.store(), PARTITION_SIZE, false, &extra_sectors(), SECTOR_SIZE, QUANT_SPEC)
        .unwrap();
    let expected_power = power_for_sectors(SECTOR_SIZE, &extra_sectors());
    assert_eq!(expected_power, power);

    let mut sectors_array = sectors_array(&rt, rt.store(), all_sectors());

    let mut post_partitions = [
        PoStPartition { index: 0, skipped: UnvalidatedBitField::Validated(BitField::default()) },
        PoStPartition { index: 1, skipped: UnvalidatedBitField::Validated(BitField::default()) },
    ];

    let post_result1 = deadline
        .record_proven_sectors(
            rt.store(),
            &sectors_array,
            SECTOR_SIZE,
            QUANT_SPEC,
            fault_expiration_epoch,
            &mut post_partitions,
        )
        .unwrap();
    assert_bitfield_equals(&post_result1.sectors, &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(post_result1.ignored_sectors.is_empty());
    assert_eq!(post_result1.new_faulty_power, PowerPair::zero());
    assert_eq!(post_result1.retracted_recovery_power, PowerPair::zero());
    assert_eq!(post_result1.recovered_power, PowerPair::zero());

    // first two partitions posted
    deadline_state()
        .with_posts(&[0, 1])
        .with_unproven(&[10])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9, 10]),
        ])
        .assert(rt.store(), &all_sectors(), &deadline);

    let mut post_partitions =
        [PoStPartition { index: 2, skipped: UnvalidatedBitField::Validated(BitField::default()) }];
    let post_result2 = deadline
        .record_proven_sectors(
            rt.store(),
            &sectors_array,
            SECTOR_SIZE,
            QUANT_SPEC,
            fault_expiration_epoch,
            &mut post_partitions,
        )
        .unwrap();
    assert_bitfield_equals(&post_result2.sectors, &[9, 10]);
    assert!(post_result2.ignored_sectors.is_empty());
    assert_eq!(post_result2.new_faulty_power, PowerPair::zero());
    assert_eq!(post_result2.retracted_recovery_power, PowerPair::zero());
    assert_eq!(post_result2.recovered_power, PowerPair::zero());

    // activate sector 10
    assert_eq!(post_result2.power_delta, sector_power(&[10]));

    // all 3 partitions posted, unproven sector 10 proven and power activated
    deadline_state()
        .with_posts(&[0, 1, 2])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9, 10]),
        ])
        .assert(rt.store(), &all_sectors(), &deadline);
    let sector_array_root = sectors_array.amt.flush().unwrap();

    let (power_delta, penalized_power) = deadline
        .process_deadline_end(rt.store(), QUANT_SPEC, fault_expiration_epoch, sector_array_root)
        .unwrap();

    // No power delta for successful post.
    assert!(power_delta.is_zero());
    assert!(penalized_power.is_zero());

    // everything back to normal
    deadline_state()
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9, 10]),
        ])
        .assert(rt.store(), &all_sectors(), &deadline);
}

#[test]
fn post_with_unproven_faults_recoveries_untracted_recoveries() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();
    let fault_expiration_epoch = 13;

    // Adds sectors 1-9 then marks sectors 1 (partition 0), 5 & 6 (partition 1) as faulty
    add_then_mark_faulty(&rt, &mut deadline, true);

    // add an inactive sector
    let power = deadline
        .add_sectors(rt.store(), PARTITION_SIZE, false, &extra_sectors(), SECTOR_SIZE, QUANT_SPEC)
        .unwrap();
    let expected_power = power_for_sectors(SECTOR_SIZE, &extra_sectors());
    assert_eq!(power, expected_power);

    let mut sectors_array = sectors_array(&rt, rt.store(), all_sectors());

    // declare sectors 1 & 6 recovered
    let mut partition_sector_map = PartitionSectorMap::default();
    partition_sector_map.add(0, UnvalidatedBitField::Validated(new_bitfield(&[1]))).unwrap();
    partition_sector_map.add(1, UnvalidatedBitField::Validated(new_bitfield(&[6]))).unwrap();
    deadline
        .declare_faults_recovered(
            rt.store(),
            &sectors_array,
            SECTOR_SIZE,
            &mut partition_sector_map,
        )
        .unwrap();

    // we are now recovering 1 & 6
    deadline_state()
        .with_recovering(&[1, 6])
        .with_faults(&[1, 5, 6])
        .with_unproven(&[10])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9, 10]),
        ])
        .assert(rt.store(), &all_sectors(), &deadline);

    // prove partitions 0 & 1, skipping sectors 1 & 7
    let mut post_partitions = [
        PoStPartition { index: 0, skipped: UnvalidatedBitField::Validated(new_bitfield(&[1])) },
        PoStPartition { index: 1, skipped: UnvalidatedBitField::Validated(new_bitfield(&[7])) },
    ];
    let post_result = deadline
        .record_proven_sectors(
            rt.store(),
            &sectors_array,
            SECTOR_SIZE,
            QUANT_SPEC,
            fault_expiration_epoch,
            &mut post_partitions,
        )
        .unwrap();

    // 1, 5, and 7 are expected to be faulty.
    // - 1 should have recovered but didn't (retracted)
    // - 5 was already marked faulty.
    // - 7 is newly faulty.
    // - 6 has recovered.
    assert_bitfield_equals(&post_result.sectors, &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_bitfield_equals(&post_result.ignored_sectors, &[1, 5, 7]);

    // sector 7 is newly faulty
    assert_eq!(post_result.new_faulty_power, sector_power(&[7]));
    // we failed to recover 1 (retracted)
    assert_eq!(post_result.retracted_recovery_power, sector_power(&[1]));
    // we recovered 6
    assert_eq!(post_result.recovered_power, sector_power(&[6]));
    // no power delta from these deadlines
    assert!(post_result.power_delta.is_zero());

    // first two partitions should be posted
    deadline_state()
        .with_posts(&[0, 1])
        .with_faults(&[1, 5, 7])
        .with_unproven(&[10])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9, 10]),
        ])
        .assert(rt.store(), &all_sectors(), &deadline);

    let sector_array_root = sectors_array.amt.flush().unwrap();
    let (power_delta, penalized_power) = deadline
        .process_deadline_end(rt.store(), QUANT_SPEC, fault_expiration_epoch, sector_array_root)
        .unwrap();

    let expected_fault_power = sector_power(&[9, 10]);
    let expected_power_delta = -sector_power(&[9]);

    // sector 9 wasn't proven
    assert_eq!(power_delta, expected_power_delta);
    // no new changes to recovering power
    assert_eq!(penalized_power, expected_fault_power);

    // posts taken care of
    // unproven now faulty
    deadline_state()
        .with_faults(&[1, 5, 7, 9, 10])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9, 10]),
        ])
        .assert(rt.store(), &all_sectors(), &deadline);
}

#[test]
fn post_with_skipped_unproven() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();
    let fault_expiration_epoch = 13;

    add_sectors(&rt, &mut deadline, true);

    // add an inactive sector
    let power = deadline
        .add_sectors(rt.store(), PARTITION_SIZE, false, &extra_sectors(), SECTOR_SIZE, QUANT_SPEC)
        .unwrap();
    let expected_power = power_for_sectors(SECTOR_SIZE, &extra_sectors());
    assert_eq!(power, expected_power);

    let mut sectors_array = sectors_array(&rt, rt.store(), all_sectors());
    let mut post_partitions = [
        PoStPartition { index: 0, skipped: UnvalidatedBitField::Validated(BitField::default()) },
        PoStPartition { index: 1, skipped: UnvalidatedBitField::Validated(BitField::default()) },
        PoStPartition { index: 2, skipped: UnvalidatedBitField::Validated(new_bitfield(&[10])) },
    ];
    let post_result = deadline
        .record_proven_sectors(
            rt.store(),
            &sectors_array,
            SECTOR_SIZE,
            QUANT_SPEC,
            fault_expiration_epoch,
            &mut post_partitions,
        )
        .unwrap();

    assert_bitfield_equals(&post_result.sectors, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    assert_bitfield_equals(&post_result.ignored_sectors, &[10]);
    assert_eq!(post_result.new_faulty_power, sector_power(&[10]));
    assert!(post_result.power_delta.is_zero());
    assert!(post_result.retracted_recovery_power.is_zero());
    assert!(post_result.recovered_power.is_zero());

    // all posted
    deadline_state()
        .with_posts(&[0, 1, 2])
        .with_faults(&[10])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9, 10]),
        ])
        .assert(rt.store(), &all_sectors(), &deadline);

    let sector_array_root = sectors_array.amt.flush().unwrap();
    let (power_delta, penalized_power) = deadline
        .process_deadline_end(rt.store(), QUANT_SPEC, fault_expiration_epoch, sector_array_root)
        .unwrap();

    // all posts submitted, no power delta, no extra penalties
    assert!(power_delta.is_zero());
    assert!(penalized_power.is_zero());

    // everything back to normal, except that we have a fault
    deadline_state()
        .with_faults(&[10])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9, 10]),
        ])
        .assert(rt.store(), &all_sectors(), &deadline);
}

#[test]
fn post_missing_partition() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();
    let fault_expiration_epoch = 13;

    add_sectors(&rt, &mut deadline, true);

    // add an inactive sector
    let power = deadline
        .add_sectors(rt.store(), PARTITION_SIZE, false, &extra_sectors(), SECTOR_SIZE, QUANT_SPEC)
        .unwrap();
    let expected_power = power_for_sectors(SECTOR_SIZE, &extra_sectors());
    assert_eq!(power, expected_power);

    let sectors_array = sectors_array(&rt, rt.store(), all_sectors());
    let mut post_partitions = [
        PoStPartition { index: 0, skipped: UnvalidatedBitField::Validated(BitField::default()) },
        PoStPartition { index: 3, skipped: UnvalidatedBitField::Validated(BitField::default()) },
    ];
    let post_result = deadline.record_proven_sectors(
        rt.store(),
        &sectors_array,
        SECTOR_SIZE,
        QUANT_SPEC,
        fault_expiration_epoch,
        &mut post_partitions,
    );

    let err = post_result
        .err()
        .expect("missing partition, should have failed")
        .downcast::<ActorError>()
        .expect("Invalid error");
    assert_eq!(err.exit_code(), ExitCode::USR_NOT_FOUND);
}

#[test]
fn post_partition_twice() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();
    let fault_expiration_epoch = 13;

    add_sectors(&rt, &mut deadline, true);

    // add an inactive sector
    let power = deadline
        .add_sectors(rt.store(), PARTITION_SIZE, false, &extra_sectors(), SECTOR_SIZE, QUANT_SPEC)
        .unwrap();
    let expected_power = power_for_sectors(SECTOR_SIZE, &extra_sectors());
    assert_eq!(power, expected_power);

    let sectors_array = sectors_array(&rt, rt.store(), all_sectors());
    let mut post_partitions = [
        PoStPartition { index: 0, skipped: UnvalidatedBitField::Validated(BitField::default()) },
        PoStPartition { index: 0, skipped: UnvalidatedBitField::Validated(BitField::default()) },
    ];
    let post_result = deadline.record_proven_sectors(
        rt.store(),
        &sectors_array,
        SECTOR_SIZE,
        QUANT_SPEC,
        fault_expiration_epoch,
        &mut post_partitions,
    );

    let err = post_result
        .err()
        .expect("duplicate partition, should have failed")
        .downcast::<ActorError>()
        .expect("Invalid error");
    assert_eq!(err.exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn retract_recoveries() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();
    let fault_expiration_epoch = 13;

    // Adds sectors 1-9 then marks sectors 1 (partition 0), 5 & 6 (partition 1) as faulty
    let (_, sectors) = add_then_mark_faulty(&rt, &mut deadline, true);

    let mut sectors_array = sectors_array(&rt, rt.store(), sectors.to_owned());

    // declare sectors 1 & 6 recovered
    let mut partition_sector_map = PartitionSectorMap::default();
    partition_sector_map.add(0, UnvalidatedBitField::Validated(new_bitfield(&[1]))).unwrap();
    partition_sector_map.add(1, UnvalidatedBitField::Validated(new_bitfield(&[6]))).unwrap();
    deadline
        .declare_faults_recovered(
            rt.store(),
            &sectors_array,
            SECTOR_SIZE,
            &mut partition_sector_map,
        )
        .unwrap();

    // retract recovery for sector 1
    let mut partition_sector_map = PartitionSectorMap::default();
    partition_sector_map.add(0, UnvalidatedBitField::Validated(new_bitfield(&[1]))).unwrap();
    let power_delta = deadline
        .record_faults(
            rt.store(),
            &sectors_array,
            SECTOR_SIZE,
            QUANT_SPEC,
            fault_expiration_epoch,
            &mut partition_sector_map,
        )
        .unwrap();

    // we're just retracting a recovery, this doesn't count as a new fault
    assert!(power_delta.is_zero());

    // we are now recovering 6
    deadline_state()
        .with_recovering(&[6])
        .with_faults(&[1, 5, 6])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(rt.store(), &sectors, &deadline);

    // prove all partitions
    let post_result = deadline
        .record_proven_sectors(
            rt.store(),
            &sectors_array,
            SECTOR_SIZE,
            QUANT_SPEC,
            fault_expiration_epoch,
            &mut [
                PoStPartition {
                    index: 0,
                    skipped: UnvalidatedBitField::Validated(BitField::default()),
                },
                PoStPartition {
                    index: 1,
                    skipped: UnvalidatedBitField::Validated(BitField::default()),
                },
                PoStPartition {
                    index: 2,
                    skipped: UnvalidatedBitField::Validated(BitField::default()),
                },
            ],
        )
        .unwrap();

    // 1 & 5 are still faulty
    assert_bitfield_equals(&post_result.sectors, &[1, 2, 3, 4, 5, 6, 7, 8, 9]);
    assert_bitfield_equals(&post_result.ignored_sectors, &[1, 5]);

    // all faults were declared
    assert!(post_result.new_faulty_power.is_zero());
    // we didn't fail to recover anything
    assert!(post_result.retracted_recovery_power.is_zero());
    // we recovered 6
    assert_eq!(post_result.recovered_power, sector_power(&[6]));

    // first three partitions should be posted
    deadline_state()
        .with_posts(&[0, 1, 2])
        .with_faults(&[1, 5])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(rt.store(), &sectors, &deadline);

    let sector_array_root = sectors_array.amt.flush().unwrap();
    let (new_faulty_power, failed_recovery_power) = deadline
        .process_deadline_end(rt.store(), QUANT_SPEC, fault_expiration_epoch, sector_array_root)
        .unwrap();

    // no power changes
    assert!(new_faulty_power.is_zero());
    assert!(failed_recovery_power.is_zero());

    // posts taken care of
    deadline_state()
        .with_faults(&[1, 5])
        .with_partitions(vec![
            new_bitfield(&[1, 2, 3, 4]),
            new_bitfield(&[5, 6, 7, 8]),
            new_bitfield(&[9]),
        ])
        .assert(rt.store(), &all_sectors(), &deadline);
}

#[test]
fn cannot_declare_faults_in_missing_partitions() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    let (_, sectors) = add_sectors(&rt, &mut deadline, true);
    let sectors_array = sectors_array(&rt, rt.store(), sectors);

    // declare sectors 1 & 6 faulty
    let mut partition_sector_map = PartitionSectorMap::default();
    partition_sector_map.add(0, UnvalidatedBitField::Validated(new_bitfield(&[1]))).unwrap();
    partition_sector_map.add(4, UnvalidatedBitField::Validated(new_bitfield(&[6]))).unwrap();
    let result = deadline.record_faults(
        rt.store(),
        &sectors_array,
        SECTOR_SIZE,
        QUANT_SPEC,
        17,
        &mut partition_sector_map,
    );

    let err = result
        .err()
        .expect("missing partition, should have failed")
        .downcast::<ActorError>()
        .expect("Invalid error");
    assert_eq!(err.exit_code(), ExitCode::USR_NOT_FOUND);
}

#[test]
fn cannot_declare_faults_recovered_in_missing_partitions() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    // Marks sectors 1 (partition 0), 5 & 6 (partition 1) as faulty.
    let (_, sectors) = add_then_mark_faulty(&rt, &mut deadline, true);
    let sectors_array = sectors_array(&rt, rt.store(), sectors);

    // declare sectors 1 & 6 recovered
    let mut partition_sector_map = PartitionSectorMap::default();
    partition_sector_map.add(0, UnvalidatedBitField::Validated(new_bitfield(&[1]))).unwrap();
    partition_sector_map.add(4, UnvalidatedBitField::Validated(new_bitfield(&[6]))).unwrap();
    let result = deadline.declare_faults_recovered(
        rt.store(),
        &sectors_array,
        SECTOR_SIZE,
        &mut partition_sector_map,
    );

    let err = result
        .err()
        .expect("missing partition, should have failed")
        .downcast::<ActorError>()
        .expect("Invalid error");
    assert_eq!(err.exit_code(), ExitCode::USR_NOT_FOUND);
}

fn deadline_state() -> ExpectedDeadlineState {
    ExpectedDeadlineState {
        quant: QUANT_SPEC,
        partition_size: PARTITION_SIZE,
        sector_size: SECTOR_SIZE,
        sectors: all_sectors(),
        ..Default::default()
    }
}

fn sector_power(sector_numbers: &[u64]) -> PowerPair {
    power_for_sectors(SECTOR_SIZE, &select_sectors(&all_sectors(), &new_bitfield(sector_numbers)))
}

fn select_sectors(sectors: &[SectorOnChainInfo], field: &BitField) -> Vec<SectorOnChainInfo> {
    let mut to_include: BTreeSet<_> = field.iter().collect();
    let included =
        sectors.iter().filter(|sector| to_include.remove(&sector.sector_number)).cloned().collect();

    assert!(to_include.is_empty(), "failed to find {} expected sectors", to_include.len());

    included
}
