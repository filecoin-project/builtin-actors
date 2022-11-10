use std::collections::HashMap;

use fil_actor_miner_state_v9::testing::{check_deadline_state_invariants, DeadlineStateSummary};
use fil_actor_miner_state_v9::{
    power_for_sectors, Deadline, PartitionSectorMap, PoStPartition, PowerPair, SectorOnChainInfo,
    TerminationResult,
};
use fil_actors_runtime_common::runtime::{Policy, Runtime};
use fil_actors_runtime_common::test_utils::MockRuntime;
use fil_actors_runtime_common::ActorError;
use fil_actors_runtime_common::MessageAccumulator;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::{clock::QuantSpec, error::ExitCode, sector::SectorSize};

mod util;
use crate::util::*;

const SECTOR_SIZE: SectorSize = SectorSize::_32GiB;
const QUANT_SPEC: QuantSpec = QuantSpec { unit: 4, offset: 1 };

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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
        ])
        .assert(store, &sectors, deadline);

    if !prove {
        return (deadline_state, sectors);
    }

    let mut sector_array = sectors_arr(store, sectors.to_owned());

    //prove everything
    let result = deadline
        .record_proven_sectors(
            store,
            &sector_array,
            SECTOR_SIZE,
            QUANT_SPEC,
            0,
            &mut [
                PoStPartition { index: 0, skipped: BitField::default() },
                PoStPartition { index: 1, skipped: BitField::default() },
                PoStPartition { index: 2, skipped: BitField::default() },
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
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
        HashMap::from([(0, bitfield_from_slice(&[1, 3])), (1, bitfield_from_slice(&[6]))]),
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
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
        .remove_partitions(store, &bitfield_from_slice(&[0]), QUANT_SPEC)
        .expect("should have removed partitions");

    assert_bitfield_equals(&live, &[2, 4]);
    assert_bitfield_equals(&dead, &[1, 3]);

    let live_power = power_for_sectors(SECTOR_SIZE, &select_sectors(&sectors, &live));
    assert_eq!(live_power, removed_power);

    let deadline_state = deadline_state
        .with_terminations(&[6])
        .with_partitions(vec![bitfield_from_slice(&[5, 6, 7, 8]), bitfield_from_slice(&[9])])
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
    let sectors_array = sectors_arr(store, sectors.to_owned());

    let mut partition_sector_map = PartitionSectorMap::default();
    partition_sector_map.add(0, bitfield_from_slice(&[1])).unwrap();
    partition_sector_map.add(1, bitfield_from_slice(&[5, 6])).unwrap();

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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
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
    assert!(deadline.remove_partitions(store, &bitfield_from_slice(&[0]), QUANT_SPEC).is_err());
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
        ])
        .assert(store, &sectors, &deadline);
}

#[test]
fn cannot_remove_missing_partition() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_then_terminate_then_remove_partition(&rt, &mut deadline);
    assert!(deadline
        .remove_partitions(rt.store(), &bitfield_from_slice(&[2]), QUANT_SPEC)
        .is_err());
}

#[test]
fn removing_no_partitions_does_nothing() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    let (deadline_state, sectors) = add_then_terminate_then_pop_early(&rt, &mut deadline);
    let (live, dead, removed_power) = deadline
        .remove_partitions(rt.store(), &bitfield_from_slice(&[]), QUANT_SPEC)
        .expect("should not have failed to remove partitions");

    assert!(removed_power.is_zero());
    assert!(live.is_empty());
    assert!(dead.is_empty());

    // Popping early terminations doesn't affect the terminations bitfield.
    deadline_state
        .with_terminations(&[1, 3, 6])
        .with_partitions(vec![
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
        ])
        .assert(rt.store(), &sectors, &deadline);
}

#[test]
fn fails_to_remove_partitions_with_faulty_sectors() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    add_then_mark_faulty(&rt, &mut deadline, false);

    // Try to remove a partition with faulty sectors.
    assert!(deadline
        .remove_partitions(rt.store(), &bitfield_from_slice(&[1]), QUANT_SPEC)
        .is_err());
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
        HashMap::from([(0, bitfield_from_slice(&[1, 3])), (1, bitfield_from_slice(&[6]))]),
    )
    .unwrap();

    // Sector 3 active, 1, 6 faulty
    let expected_power_loss =
        power_for_sectors(SECTOR_SIZE, &select_sectors(&sectors, &bitfield_from_slice(&[3])));
    assert_eq!(
        expected_power_loss, removed_power,
        "deadline state to remove power for terminated sectors"
    );

    deadline_state
        .with_terminations(&[1, 3, 6])
        .with_faults(&[5])
        .with_partitions(vec![
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
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
    let sectors_array = sectors_arr(&store, sectors);

    let mut partition_sector_map = PartitionSectorMap::default();
    for (partition, sectors) in partition_sectors {
        partition_sector_map.add(partition, sectors).unwrap();
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
        HashMap::from([(0, bitfield_from_slice(&[1, 3])), (1, bitfield_from_slice(&[6]))]),
    )
    .unwrap();

    // sector 3 unproven, 1, 6 faulty
    assert!(removed_power.is_zero(), "should remove no power");

    deadline_state
        .with_terminations(&[1, 3, 6])
        .with_faults(&[5])
        .with_unproven(&[2, 4, 7, 8, 9]) // not 1, 3, 5, & 6
        .with_partitions(vec![
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
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
        HashMap::from([(0, bitfield_from_slice(&[6]))]),
    );

    assert!(ret.is_err());
    let err = ret
        .expect_err("can only terminate live sectors")
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
        HashMap::from([(4, bitfield_from_slice(&[6]))]),
    );

    assert!(ret.is_err());
    let err = ret
        .expect_err("can only terminate existing partitions")
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
        HashMap::from([(0, bitfield_from_slice(&[1, 2]))]),
    );

    assert!(ret.is_err());
    let err = ret
        .expect_err("cannot terminate already terminated sector")
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
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
    let err = ret.expect_err("cannot pop expired sectors from a partition with unproven sectors");

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

    let mut sectors_array = sectors_arr(rt.store(), all_sectors());

    let mut post_partitions = [
        PoStPartition { index: 0, skipped: BitField::default() },
        PoStPartition { index: 1, skipped: BitField::default() },
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9, 10]),
        ])
        .assert(rt.store(), &all_sectors(), &deadline);

    let mut post_partitions = [PoStPartition { index: 2, skipped: BitField::default() }];
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9, 10]),
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9, 10]),
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

    let mut sectors_array = sectors_arr(rt.store(), all_sectors());

    // declare sectors 1 & 6 recovered
    let mut partition_sector_map = PartitionSectorMap::default();
    partition_sector_map.add(0, bitfield_from_slice(&[1])).unwrap();
    partition_sector_map.add(1, bitfield_from_slice(&[6])).unwrap();
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9, 10]),
        ])
        .assert(rt.store(), &all_sectors(), &deadline);

    // prove partitions 0 & 1, skipping sectors 1 & 7
    let mut post_partitions = [
        PoStPartition { index: 0, skipped: bitfield_from_slice(&[1]) },
        PoStPartition { index: 1, skipped: bitfield_from_slice(&[7]) },
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9, 10]),
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9, 10]),
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

    let mut sectors_array = sectors_arr(rt.store(), all_sectors());
    let mut post_partitions = [
        PoStPartition { index: 0, skipped: BitField::default() },
        PoStPartition { index: 1, skipped: BitField::default() },
        PoStPartition { index: 2, skipped: bitfield_from_slice(&[10]) },
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9, 10]),
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9, 10]),
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

    let sectors_array = sectors_arr(rt.store(), all_sectors());
    let mut post_partitions = [
        PoStPartition { index: 0, skipped: BitField::default() },
        PoStPartition { index: 3, skipped: BitField::default() },
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

    let sectors_array = sectors_arr(rt.store(), all_sectors());
    let mut post_partitions = [
        PoStPartition { index: 0, skipped: BitField::default() },
        PoStPartition { index: 0, skipped: BitField::default() },
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

    let mut sectors_array = sectors_arr(rt.store(), sectors.to_owned());

    // declare sectors 1 & 6 recovered
    let mut partition_sector_map = PartitionSectorMap::default();
    partition_sector_map.add(0, bitfield_from_slice(&[1])).unwrap();
    partition_sector_map.add(1, bitfield_from_slice(&[6])).unwrap();
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
    partition_sector_map.add(0, bitfield_from_slice(&[1])).unwrap();
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
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
                PoStPartition { index: 0, skipped: BitField::default() },
                PoStPartition { index: 1, skipped: BitField::default() },
                PoStPartition { index: 2, skipped: BitField::default() },
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
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
            bitfield_from_slice(&[1, 2, 3, 4]),
            bitfield_from_slice(&[5, 6, 7, 8]),
            bitfield_from_slice(&[9]),
        ])
        .assert(rt.store(), &all_sectors(), &deadline);
}

#[test]
fn cannot_declare_faults_in_missing_partitions() {
    let (_, rt) = setup();
    let mut deadline = Deadline::new(rt.store()).unwrap();

    let (_, sectors) = add_sectors(&rt, &mut deadline, true);
    let sectors_array = sectors_arr(rt.store(), sectors);

    // declare sectors 1 & 6 faulty
    let mut partition_sector_map = PartitionSectorMap::default();
    partition_sector_map.add(0, bitfield_from_slice(&[1])).unwrap();
    partition_sector_map.add(4, bitfield_from_slice(&[6])).unwrap();
    let result = deadline.record_faults(
        rt.store(),
        &sectors_array,
        SECTOR_SIZE,
        QUANT_SPEC,
        17,
        &mut partition_sector_map,
    );

    let err = result
        .expect_err("missing partition, should have failed")
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
    let sectors_array = sectors_arr(rt.store(), sectors);

    // declare sectors 1 & 6 recovered
    let mut partition_sector_map = PartitionSectorMap::default();
    partition_sector_map.add(0, bitfield_from_slice(&[1])).unwrap();
    partition_sector_map.add(4, bitfield_from_slice(&[6])).unwrap();
    let result = deadline.declare_faults_recovered(
        rt.store(),
        &sectors_array,
        SECTOR_SIZE,
        &mut partition_sector_map,
    );

    let err = result
        .expect_err("missing partition, should have failed")
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
    power_for_sectors(
        SECTOR_SIZE,
        &select_sectors(&all_sectors(), &bitfield_from_slice(sector_numbers)),
    )
}

fn make_bitfield(sector_numbers: &[u64]) -> BitField {
    BitField::try_from_bits(sector_numbers.iter().copied()).unwrap()
}

// Helper type for validating deadline state.
//
// All methods take the state by value so one can (and should) construct a
// sane base-state.
struct ExpectedDeadlineState {
    quant: QuantSpec,
    #[allow(dead_code)]
    sector_size: SectorSize,
    #[allow(dead_code)]
    partition_size: u64,
    #[allow(dead_code)]
    sectors: Vec<SectorOnChainInfo>,
    faults: BitField,
    recovering: BitField,
    terminations: BitField,
    unproven: BitField,
    posts: BitField,
    partition_sectors: Vec<BitField>,
}

impl Default for ExpectedDeadlineState {
    fn default() -> Self {
        Self {
            quant: QuantSpec { offset: 0, unit: 0 },
            sector_size: SectorSize::_32GiB,
            partition_size: 0,
            sectors: vec![],
            faults: BitField::default(),
            recovering: BitField::default(),
            terminations: BitField::default(),
            unproven: BitField::default(),
            posts: BitField::default(),
            partition_sectors: vec![],
        }
    }
}

impl ExpectedDeadlineState {
    #[allow(dead_code)]
    fn with_quant_spec(mut self, quant: QuantSpec) -> Self {
        self.quant = quant;
        self
    }

    fn with_faults(mut self, faults: &[u64]) -> Self {
        self.faults = make_bitfield(faults);
        self
    }

    #[allow(dead_code)]
    fn with_recovering(mut self, recovering: &[u64]) -> Self {
        self.recovering = make_bitfield(recovering);
        self
    }

    fn with_terminations(mut self, terminations: &[u64]) -> Self {
        self.terminations = make_bitfield(terminations);
        self
    }

    fn with_unproven(mut self, unproven: &[u64]) -> Self {
        self.unproven = make_bitfield(unproven);
        self
    }

    #[allow(dead_code)]
    fn with_posts(mut self, posts: &[u64]) -> Self {
        self.posts = make_bitfield(posts);
        self
    }

    fn with_partitions(mut self, partitions: Vec<BitField>) -> Self {
        self.partition_sectors = partitions;
        self
    }

    // Assert that the deadline's state matches the expected state.
    fn assert<BS: Blockstore>(
        self,
        store: &BS,
        sectors: &[SectorOnChainInfo],
        deadline: &Deadline,
    ) -> Self {
        let summary = self.check_deadline_invariants(store, sectors, deadline);

        assert_eq!(self.faults, summary.faulty_sectors);
        assert_eq!(self.recovering, summary.recovering_sectors);
        assert_eq!(self.terminations, summary.terminated_sectors);
        assert_eq!(self.unproven, summary.unproven_sectors);
        assert_eq!(self.posts, deadline.partitions_posted);

        let partitions = deadline.partitions_amt(store).unwrap();
        assert_eq!(self.partition_sectors.len() as u64, partitions.count());

        for (i, partition_sectors) in self.partition_sectors.iter().enumerate() {
            let partitions = partitions.get(i as u64).unwrap().unwrap();
            assert_eq!(partition_sectors, &partitions.sectors);
        }

        self
    }

    // check the deadline's invariants, returning all contained sectors, faults,
    // recoveries, terminations, and partition/sector assignments.
    fn check_deadline_invariants<BS: Blockstore>(
        &self,
        store: &BS,
        sectors: &[SectorOnChainInfo],
        deadline: &Deadline,
    ) -> DeadlineStateSummary {
        let acc = MessageAccumulator::default();
        let summary = check_deadline_state_invariants(
            deadline,
            store,
            QUANT_SPEC,
            SECTOR_SIZE,
            &sectors_as_map(sectors),
            &acc,
        );

        acc.assert_empty();

        summary
    }
}
