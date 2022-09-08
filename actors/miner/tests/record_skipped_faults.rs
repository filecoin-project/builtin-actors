use fil_actor_miner::power_for_sectors;
use fil_actor_miner::select_sectors;
use fil_actor_miner::testing::PartitionStateSummary;
use fil_actor_miner::Partition;
use fil_actor_miner::SectorOnChainInfo;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::ActorError;
use fil_actors_runtime::MessageAccumulator;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::clock::QuantSpec;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::SectorSize;
use std::ops::Neg;

mod util;
use crate::util::*;

fn sectors() -> Vec<SectorOnChainInfo> {
    vec![
        test_sector(2, 1, 50, 60, 1000),
        test_sector(3, 2, 51, 61, 1001),
        test_sector(7, 3, 52, 62, 1002),
        test_sector(8, 4, 53, 63, 1003),
        test_sector(11, 5, 54, 64, 1004),
        test_sector(13, 6, 55, 65, 1005),
    ]
}

const SECTOR_SIZE: SectorSize = SectorSize::_32GiB;
const QUANT_SPEC: QuantSpec = QuantSpec { unit: 4, offset: 1 };
const EXP: ChainEpoch = 100;

fn setup() -> (MemoryBlockstore, Partition) {
    let store = MemoryBlockstore::default();
    let mut partition = Partition::new(&store).unwrap();

    let power = partition.add_sectors(&store, true, &sectors(), SECTOR_SIZE, QUANT_SPEC).unwrap();
    let expected_power = power_for_sectors(SECTOR_SIZE, &sectors());
    assert_eq!(expected_power, power);
    (store, partition)
}

#[test]
fn fail_if_all_declared_sectors_are_not_in_the_partition() {
    let (store, mut partition) = setup();
    let sector_arr = sectors_arr_mbs(&store, sectors());

    let mut skipped = BitField::try_from_bits(1..100).unwrap().into();

    let err: ActorError = partition
        .record_skipped_faults(&store, &sector_arr, SECTOR_SIZE, QUANT_SPEC, EXP, &mut skipped)
        .unwrap_err()
        .downcast()
        .unwrap();
    assert_eq!(ExitCode::USR_ILLEGAL_ARGUMENT, err.exit_code());
}

#[test]
fn already_faulty_and_terminated_sectors_are_ignored() {
    let policy = Policy::default();
    let (store, mut partition) = setup();
    let sector_arr = sectors_arr_mbs(&store, sectors());

    // terminate 1 AND 2
    let terminations: BitField = BitField::try_from_bits([1, 2]).unwrap();
    let termination_epoch = 3;
    partition
        .terminate_sectors(
            &policy,
            &store,
            &sector_arr,
            termination_epoch,
            &mut (terminations.clone().into()),
            SECTOR_SIZE,
            QUANT_SPEC,
        )
        .unwrap();
    assert_partition_state(
        &store,
        &partition,
        QUANT_SPEC,
        SECTOR_SIZE,
        &sectors(),
        &BitField::try_from_bits(1..=6).unwrap(),
        &BitField::new(),
        &BitField::new(),
        &terminations,
        &BitField::new(),
    );

    // declare 4 & 5 as faulty
    let fault_set = BitField::try_from_bits([4, 5]).unwrap();
    partition
        .record_faults(
            &store,
            &sector_arr,
            &mut fault_set.clone().into(),
            7,
            SECTOR_SIZE,
            QUANT_SPEC,
        )
        .unwrap();
    assert_partition_state(
        &store,
        &partition,
        QUANT_SPEC,
        SECTOR_SIZE,
        &sectors(),
        &BitField::try_from_bits(1..=6).unwrap(),
        &fault_set,
        &BitField::new(),
        &terminations,
        &BitField::new(),
    );

    // record skipped faults such that some of them are already faulty/terminated
    let skipped = BitField::try_from_bits([1, 2, 3, 4, 5]).unwrap();
    let (power_delta, new_fault_power, retracted_power, new_faults) = partition
        .record_skipped_faults(
            &store,
            &sector_arr,
            SECTOR_SIZE,
            QUANT_SPEC,
            EXP,
            &mut skipped.into(),
        )
        .unwrap();
    assert!(retracted_power.is_zero());
    let expected_faulty_power = power_for_sectors(
        SECTOR_SIZE,
        &select_sectors(&sectors(), &BitField::try_from_bits([3]).unwrap()).unwrap(),
    );
    assert_eq!(expected_faulty_power, new_fault_power);
    assert_eq!(power_delta, new_fault_power.neg());
    assert!(new_faults);

    assert_partition_state(
        &store,
        &partition,
        QUANT_SPEC,
        SECTOR_SIZE,
        &sectors(),
        &BitField::try_from_bits(1..=6).unwrap(),
        &BitField::try_from_bits([3, 4, 5]).unwrap(),
        &BitField::new(),
        &BitField::try_from_bits([1, 2]).unwrap(),
        &BitField::new(),
    );
}

#[test]
fn recoveries_are_retracted_without_being_marked_as_new_faulty_power() {
    let (store, mut partition) = setup();
    let sector_arr = sectors_arr_mbs(&store, sectors());

    // make 4, 5 and 6 faulty
    let fault_set = BitField::try_from_bits([4, 5, 6]).unwrap();
    let _ = partition
        .record_faults(&store, &sector_arr, &mut fault_set.into(), 7, SECTOR_SIZE, QUANT_SPEC)
        .unwrap();

    // add 4 and 5 as recoveries
    let recover_set = BitField::try_from_bits([4, 5]).unwrap();
    partition.declare_faults_recovered(&sector_arr, SECTOR_SIZE, &mut recover_set.into()).unwrap();

    assert_partition_state(
        &store,
        &partition,
        QUANT_SPEC,
        SECTOR_SIZE,
        &sectors(),
        &BitField::try_from_bits(1..=6).unwrap(),
        &BitField::try_from_bits([4, 5, 6]).unwrap(),
        &BitField::try_from_bits([4, 5]).unwrap(),
        &BitField::new(),
        &BitField::new(),
    );

    // record skipped faults such that some of them have been marked as recovered
    let skipped = BitField::try_from_bits([1, 4, 5]).unwrap();
    let (power_delta, new_fault_power, recovery_power, new_faults) = partition
        .record_skipped_faults(
            &store,
            &sector_arr,
            SECTOR_SIZE,
            QUANT_SPEC,
            EXP,
            &mut skipped.into(),
        )
        .unwrap();
    assert!(new_faults);

    // only 1 is marked for fault power as 4 & 5 are recovering
    let expected_faulty_power = power_for_sectors(
        SECTOR_SIZE,
        &select_sectors(&sectors(), &BitField::try_from_bits([1]).unwrap()).unwrap(),
    );
    assert_eq!(expected_faulty_power, new_fault_power);
    assert_eq!(expected_faulty_power.neg(), power_delta);

    // 4 & 5 are marked for recovery power
    let expected_recovery_power = power_for_sectors(
        SECTOR_SIZE,
        &select_sectors(&sectors(), &BitField::try_from_bits([4, 5]).unwrap()).unwrap(),
    );
    assert_eq!(expected_recovery_power, recovery_power);

    assert_partition_state(
        &store,
        &partition,
        QUANT_SPEC,
        SECTOR_SIZE,
        &sectors(),
        &BitField::try_from_bits(1..=6).unwrap(),
        &BitField::try_from_bits([1, 4, 5, 6]).unwrap(),
        &BitField::new(),
        &BitField::new(),
        &BitField::new(),
    );
}

#[test]
fn successful_when_skipped_fault_set_is_empty() {
    let (store, mut partition) = setup();
    let sector_arr = sectors_arr_mbs(&store, sectors());

    let (power_delta, new_fault_power, recovery_power, new_faults) = partition
        .record_skipped_faults(
            &store,
            &sector_arr,
            SECTOR_SIZE,
            QUANT_SPEC,
            EXP,
            &mut BitField::new().into(),
        )
        .unwrap();
    assert!(new_fault_power.is_zero());
    assert!(recovery_power.is_zero());
    assert!(power_delta.is_zero());
    assert!(!new_faults);

    assert_partition_state(
        &store,
        &partition,
        QUANT_SPEC,
        SECTOR_SIZE,
        &sectors(),
        &BitField::try_from_bits(1..=6).unwrap(),
        &BitField::new(),
        &BitField::new(),
        &BitField::new(),
        &BitField::new(),
    );
}

#[allow(clippy::too_many_arguments)]
fn assert_partition_state(
    store: &MemoryBlockstore,
    partition: &Partition,
    quant: QuantSpec,
    sector_size: SectorSize,
    sectors: &[SectorOnChainInfo],
    all_sector_ids: &BitField,
    faults: &BitField,
    recovering: &BitField,
    terminations: &BitField,
    unproven: &BitField,
) {
    assert_eq!(faults, &partition.faults);
    assert_eq!(recovering, &partition.recoveries);
    assert_eq!(terminations, &partition.terminated);
    assert_eq!(unproven, &partition.unproven);
    assert_eq!(all_sector_ids, &partition.sectors);

    let msgs = MessageAccumulator::default();
    PartitionStateSummary::check_partition_state_invariants(
        partition,
        store,
        quant,
        sector_size,
        &sectors_as_map(sectors),
        &msgs,
    );
    msgs.assert_empty();
}
