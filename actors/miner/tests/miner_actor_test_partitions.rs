use fil_actor_miner::{
    power_for_sectors, testing::PartitionStateSummary, BitFieldQueue, ExpirationQueue, Partition,
    PowerPair, SectorOnChainInfo,
};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::MessageAccumulator;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::clock::{ChainEpoch, QuantSpec};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredSealProof, SectorSize};

mod util;
use util::*;

const SECTOR_SIZE: SectorSize = SectorSize::_32GiB;
const QUANT_SPEC: QuantSpec = QuantSpec { unit: 4, offset: 1 };

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

#[allow(clippy::too_many_arguments)]
fn assert_partition_state(
    store: &MemoryBlockstore,
    partition: &Partition,
    sectors: &[SectorOnChainInfo],
    all_sector_ids: BitField,
    faults: BitField,
    recovering: BitField,
    terminations: BitField,
    unproven: BitField,
) {
    assert_eq!(faults, partition.faults);
    assert_eq!(recovering, partition.recoveries);
    assert_eq!(terminations, partition.terminated);
    assert_eq!(unproven, partition.unproven);
    assert_eq!(all_sector_ids, partition.sectors);

    let acc = MessageAccumulator::default();
    let _ = PartitionStateSummary::check_partition_state_invariants(
        partition,
        store,
        QUANT_SPEC,
        SECTOR_SIZE,
        &sectors_as_map(sectors),
        &acc,
    );

    assert!(acc.is_empty(), "{}", acc.messages().join("\n"));
}

fn setup_unproven() -> (MockRuntime, Partition) {
    let (_, rt) = setup();
    let mut partition = Partition::new(&rt.store).unwrap();

    let power =
        partition.add_sectors(&rt.store, false, &sectors(), SECTOR_SIZE, QUANT_SPEC).unwrap();

    let expected_power = power_for_sectors(SECTOR_SIZE, &sectors());
    assert_eq!(expected_power, power);

    (rt, partition)
}

fn setup_partition() -> (MockRuntime, Partition) {
    let (rt, mut partition) = setup_unproven();

    let power = partition.activate_unproven();

    let expected_power = power_for_sectors(SECTOR_SIZE, &sectors());
    assert_eq!(expected_power, power);

    (rt, partition)
}

struct ExpectExpirationGroup {
    expiration: ChainEpoch,
    sectors: BitField,
}

fn assert_partition_expiration_queue(
    store: &MemoryBlockstore,
    partition: &Partition,
    quant: QuantSpec,
    groups: &[ExpectExpirationGroup],
) {
    let mut queue = ExpirationQueue::new(store, &partition.expirations_epochs, quant).unwrap();

    for group in groups {
        require_no_expiration_groups_before(group.expiration, &mut queue);
        let set = queue.pop_until(group.expiration).unwrap();

        // We only care whether the sectors are in the queue or not. ExpirationQueue tests can deal with early or on time.
        let all_sectors = &set.on_time_sectors | &set.early_sectors;
        assert_eq!(group.sectors, all_sectors);
    }
}

mod miner_actor_test_partitions {
    use super::*;

    #[test]
    fn adds_sectors_then_activates_unproven() {
        let (_, mut partition) = setup_unproven();

        let power = partition.activate_unproven();
        let expected_power = power_for_sectors(SECTOR_SIZE, &sectors());
        assert_eq!(expected_power, power);
    }

    #[test]
    fn adds_sectors_and_reports_sector_stats() {
        let (rt, partition) = setup_partition();

        assert_partition_state(
            &rt.store,
            &partition,
            &sectors(),
            bitfield_from_slice(&[1, 2, 3, 4, 5, 6]),
            BitField::new(),
            BitField::new(),
            BitField::new(),
            BitField::new(),
        );

        // assert sectors have been arranged into 3 groups
        assert_partition_expiration_queue(
            &rt.store,
            &partition,
            QUANT_SPEC,
            &[
                ExpectExpirationGroup { expiration: 5, sectors: bitfield_from_slice(&[1, 2]) },
                ExpectExpirationGroup { expiration: 9, sectors: bitfield_from_slice(&[3, 4]) },
                ExpectExpirationGroup { expiration: 13, sectors: bitfield_from_slice(&[5, 6]) },
            ],
        );
    }

    #[test]
    fn does_not_add_sectors_twice() {
        let (rt, mut partition) = setup_partition();

        let res = partition.add_sectors(&rt.store, false, &sectors(), SECTOR_SIZE, QUANT_SPEC);

        let err = res.expect_err("expected error, but call succeeded");
        // XXX: This is not a good way to check for specific errors.
        //      See: https://github.com/filecoin-project/builtin-actors/issues/338
        assert_eq!(err.to_string(), "not all added sectors are new");
    }

    fn assert_adds_faults(proven: bool) {
        let (rt, mut partition) = setup_unproven();
        if proven {
            partition.activate_unproven();
        }
        let sector_arr = sectors_arr(&rt.store, sectors());

        let fault_set = make_bitfield(&[4, 5]);
        let (_, power_delta, new_faulty_power) = partition
            .record_faults(&rt.store, &sector_arr, &fault_set, 7, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        let expected_faulty_power =
            power_for_sectors(SECTOR_SIZE, &select_sectors(&sectors(), &fault_set));
        let expected_power_delta =
            if proven { -expected_faulty_power.clone() } else { PowerPair::zero() };
        assert_eq!(new_faulty_power, expected_faulty_power);
        assert_eq!(power_delta, expected_power_delta);

        let sector_numbers = bitfield_from_slice(&[1, 2, 3, 4, 5, 6]);
        let unproven_set = if proven {
            bitfield_from_slice(&[])
        } else {
            bitfield_from_slice(&[1, 2, 3, 6]) // faults are no longer "unproven", just faulty.
        };

        assert_partition_state(
            &rt.store,
            &partition,
            &sectors(),
            sector_numbers,
            fault_set,
            BitField::new(),
            BitField::new(),
            unproven_set,
        );

        // moves faulty sectors after expiration to earlier group
        assert_partition_expiration_queue(
            &rt.store,
            &partition,
            QUANT_SPEC,
            &[
                ExpectExpirationGroup { expiration: 5, sectors: bitfield_from_slice(&[1, 2]) },
                ExpectExpirationGroup { expiration: 9, sectors: bitfield_from_slice(&[3, 4, 5]) },
                ExpectExpirationGroup { expiration: 13, sectors: bitfield_from_slice(&[6]) },
            ],
        );
    }

    #[test]
    fn adds_faults_proven() {
        assert_adds_faults(true)
    }

    #[test]
    fn adds_faults_unproven() {
        assert_adds_faults(false)
    }

    #[test]
    fn re_adding_faults_is_a_no_op() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        let fault_set = make_bitfield(&[4, 5]);
        let (_, power_delta, new_faulty_power) = partition
            .record_faults(&rt.store, &sector_arr, &fault_set, 7, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        let expected_faulty_power =
            power_for_sectors(SECTOR_SIZE, &select_sectors(&sectors(), &fault_set));
        assert_eq!(expected_faulty_power, new_faulty_power);
        assert_eq!(power_delta, -expected_faulty_power);

        let fault_set = make_bitfield(&[5, 6]);
        let (new_fault, power_delta, new_faulty_power) = partition
            .record_faults(&rt.store, &sector_arr, &fault_set, 3, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();
        assert_eq!(new_fault, bitfield_from_slice(&[6]));

        let expected_faulty_power =
            power_for_sectors(SECTOR_SIZE, &select_sectors(&sectors(), &bitfield_from_slice(&[6])));
        assert_eq!(expected_faulty_power, new_faulty_power);
        assert_eq!(power_delta, -expected_faulty_power);

        assert_partition_state(
            &rt.store,
            &partition,
            &sectors(),
            bitfield_from_slice(&[1, 2, 3, 4, 5, 6]),
            bitfield_from_slice(&[4, 5, 6]),
            BitField::new(),
            BitField::new(),
            BitField::new(),
        );

        // moves newly-faulty sector
        assert_partition_expiration_queue(
            &rt.store,
            &partition,
            QUANT_SPEC,
            &[
                ExpectExpirationGroup { expiration: 5, sectors: bitfield_from_slice(&[1, 2, 6]) },
                ExpectExpirationGroup { expiration: 9, sectors: bitfield_from_slice(&[3, 4, 5]) },
            ],
        );
    }

    #[test]
    fn fails_to_add_faults_for_missing_sectors() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        let fault_set = make_bitfield(&[99]);
        let res =
            partition.record_faults(&rt.store, &sector_arr, &fault_set, 7, SECTOR_SIZE, QUANT_SPEC);

        let err = res.expect_err("expected error, but call succeeded");
        // XXX: This is not a good way to check for specific errors.
        //      See: https://github.com/filecoin-project/builtin-actors/issues/338
        assert!(err.to_string().contains("not all sectors are assigned to the partition"));
    }

    #[test]
    fn adds_recoveries() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        // make 4, 5 and 6 faulty
        let fault_set = make_bitfield(&[4, 5, 6]);
        partition
            .record_faults(&rt.store, &sector_arr, &fault_set, 7, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        // add 4 and 5 as recoveries
        let recover_set = make_bitfield(&[4, 5]);
        partition.declare_faults_recovered(&sector_arr, SECTOR_SIZE, &recover_set).unwrap();

        assert_partition_state(
            &rt.store,
            &partition,
            &sectors(),
            bitfield_from_slice(&[1, 2, 3, 4, 5, 6]),
            bitfield_from_slice(&[4, 5, 6]),
            bitfield_from_slice(&[4, 5]),
            BitField::new(),
            BitField::new(),
        );
    }

    #[test]
    fn remove_recoveries() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        // make 4, 5 and 6 faulty
        let fault_set = make_bitfield(&[4, 5, 6]);
        partition
            .record_faults(&rt.store, &sector_arr, &fault_set, 7, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        // add 4 and 5 as recoveries
        let recover_set = make_bitfield(&[4, 5]);
        partition.declare_faults_recovered(&sector_arr, SECTOR_SIZE, &recover_set).unwrap();

        // declaring no faults doesn't do anything.
        let (new_faults, _, _) = partition
            .record_faults(
                &rt.store,
                &sector_arr,
                &make_empty_bitfield(),
                7,
                SECTOR_SIZE,
                QUANT_SPEC,
            )
            .unwrap();
        assert!(new_faults.is_empty()); // no new faults.

        assert_partition_state(
            &rt.store,
            &partition,
            &sectors(),
            bitfield_from_slice(&[1, 2, 3, 4, 5, 6]),
            bitfield_from_slice(&[4, 5, 6]),
            bitfield_from_slice(&[4, 5]),
            BitField::new(),
            BitField::new(),
        );

        // removing sector 5 alters recovery set and recovery power
        let (new_faults, _, _) = partition
            .record_faults(
                &rt.store,
                &sector_arr,
                &make_bitfield(&[5]),
                10,
                SECTOR_SIZE,
                QUANT_SPEC,
            )
            .unwrap();
        assert!(new_faults.is_empty()); // these faults aren't new.

        assert_partition_state(
            &rt.store,
            &partition,
            &sectors(),
            bitfield_from_slice(&[1, 2, 3, 4, 5, 6]),
            bitfield_from_slice(&[4, 5, 6]),
            bitfield_from_slice(&[4]),
            BitField::new(),
            BitField::new(),
        );
    }

    #[test]
    fn recovers_faults() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        // make 4, 5 and 6 faulty
        let fault_set = make_bitfield(&[4, 5, 6]);
        partition
            .record_faults(&rt.store, &sector_arr, &fault_set, 7, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        // add 4 and 5 as recoveries
        let recover_set = make_bitfield(&[4, 5]);
        let expected_recovery_power =
            power_for_sectors(SECTOR_SIZE, &select_sectors(&sectors(), &recover_set));
        partition.declare_faults_recovered(&sector_arr, SECTOR_SIZE, &recover_set).unwrap();

        // mark recoveries as recovered recover sectors
        let recovered_power =
            partition.recover_faults(&rt.store, &sector_arr, SECTOR_SIZE, QUANT_SPEC).unwrap();

        // recovered power should equal power of recovery sectors
        assert_eq!(expected_recovery_power, recovered_power);

        // state should be as if recovered sectors were never faults
        assert_partition_state(
            &rt.store,
            &partition,
            &sectors(),
            bitfield_from_slice(&[1, 2, 3, 4, 5, 6]),
            bitfield_from_slice(&[6]),
            BitField::new(),
            BitField::new(),
            BitField::new(),
        );

        // restores recovered expirations to original state (unrecovered sector 6 still expires early)
        assert_partition_expiration_queue(
            &rt.store,
            &partition,
            QUANT_SPEC,
            &[
                ExpectExpirationGroup { expiration: 5, sectors: bitfield_from_slice(&[1, 2]) },
                ExpectExpirationGroup { expiration: 9, sectors: bitfield_from_slice(&[3, 4, 6]) },
                ExpectExpirationGroup { expiration: 13, sectors: bitfield_from_slice(&[5]) },
            ],
        );
    }

    #[test]
    fn faulty_power_recovered_exactly_once() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        // make 4, 5 and 6 faulty
        let fault_set = make_bitfield(&[4, 5, 6]);
        partition
            .record_faults(&rt.store, &sector_arr, &fault_set, 7, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        // add 3, 4 and 5 as recoveries. 3 is not faulty so it's skipped
        let recover_set = make_bitfield(&[3, 4, 5]);
        partition.declare_faults_recovered(&sector_arr, SECTOR_SIZE, &recover_set).unwrap();

        let recovering_power =
            power_for_sectors(SECTOR_SIZE, &select_sectors(&sectors(), &fault_set));
        partition.declare_faults_recovered(&sector_arr, SECTOR_SIZE, &fault_set).unwrap();
        assert_eq!(partition.recovering_power, recovering_power);
    }

    #[test]
    fn missing_sectors_are_not_recovered() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        // try to add 99 as a recovery but it's not in the partition
        let res =
            partition.declare_faults_recovered(&sector_arr, SECTOR_SIZE, &make_bitfield(&[99]));

        let err = res.expect_err("expected error, but call succeeded");
        // XXX: This is not a good way to check for specific errors.
        //      See: https://github.com/filecoin-project/builtin-actors/issues/338
        assert!(err.to_string().contains("not all sectors are assigned to the partition"));
    }

    #[test]
    fn replace_sectors() {
        let (rt, mut partition) = setup_partition();

        // remove 3 sectors starting with 2
        let old_sectors = sectors()[1..4].to_vec();
        let old_sector_power = power_for_sectors(SECTOR_SIZE, &old_sectors);
        let old_sector_pledge = TokenAmount::from_atto(1001 + 1002 + 1003);

        // replace 1 and add 2 new sectors
        let new_sectors = vec![
            test_sector(10, 2, 150, 260, 3000),
            test_sector(10, 7, 151, 261, 3001),
            test_sector(18, 8, 152, 262, 3002),
        ];
        let new_sector_power = power_for_sectors(SECTOR_SIZE, &new_sectors);
        let new_sector_pledge = TokenAmount::from_atto(3000u64 + 3001 + 3002);

        let (power_delta, pledge_delta) = partition
            .replace_sectors(&rt.store, &old_sectors, &new_sectors, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        let expected_power_delta = new_sector_power - old_sector_power;
        assert_eq!(expected_power_delta, power_delta);
        assert_eq!(new_sector_pledge - old_sector_pledge, pledge_delta);

        // partition state should contain new sectors and not old sectors
        let mut all_sectors = new_sectors.clone();
        all_sectors.extend_from_slice(&sectors()[0..1]);
        all_sectors.extend_from_slice(&sectors()[4..]);
        assert_partition_state(
            &rt.store,
            &partition,
            &all_sectors,
            bitfield_from_slice(&[1, 2, 5, 6, 7, 8]),
            BitField::new(),
            BitField::new(),
            BitField::new(),
            BitField::new(),
        );

        // sector 2 should be moved, 3 and 4 should be removed, and 7 and 8 added
        assert_partition_expiration_queue(
            &rt.store,
            &partition,
            QUANT_SPEC,
            &[
                ExpectExpirationGroup { expiration: 5, sectors: bitfield_from_slice(&[1]) },
                ExpectExpirationGroup {
                    expiration: 13,
                    sectors: bitfield_from_slice(&[2, 5, 6, 7]),
                },
                ExpectExpirationGroup { expiration: 21, sectors: bitfield_from_slice(&[8]) },
            ],
        );
    }

    #[test]
    fn replace_sectors_errors_when_attempting_to_replace_inactive_sector() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        // fault sector 2
        let fault_set = make_bitfield(&[2]);
        partition
            .record_faults(&rt.store, &sector_arr, &fault_set, 7, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        // remove 3 sectors starting with 2
        let old_sectors = sectors()[1..4].to_vec();

        // replace sector 2
        let new_sectors = vec![test_sector(10, 2, 150, 260, 3000)];

        let res = partition.replace_sectors(
            &rt.store,
            &old_sectors,
            &new_sectors,
            SECTOR_SIZE,
            QUANT_SPEC,
        );

        let err = res.expect_err("expected error, but call succeeded");
        // XXX: This is not a good way to check for specific errors.
        //      See: https://github.com/filecoin-project/builtin-actors/issues/338
        assert!(err.to_string().contains("refusing to replace inactive sectors"));
    }

    #[test]
    fn replace_sectors_errors_when_attempting_to_unproven_sector() {
        let (rt, mut partition) = setup_unproven();

        // remove 3 sectors starting with 2
        let old_sectors = sectors()[1..4].to_vec();

        // replace sector 2
        let new_sectors = vec![test_sector(10, 2, 150, 260, 3000)];

        let res = partition.replace_sectors(
            &rt.store,
            &old_sectors,
            &new_sectors,
            SECTOR_SIZE,
            QUANT_SPEC,
        );

        let err = res.expect_err("expected error, but call succeeded");
        // XXX: This is not a good way to check for specific errors.
        //      See: https://github.com/filecoin-project/builtin-actors/issues/338
        assert!(err.to_string().contains("refusing to replace inactive sectors"));
    }

    #[test]
    fn terminate_sectors() {
        let (rt, mut partition) = setup_partition();

        let unproven_sector = vec![test_sector(13, 7, 55, 65, 1006)];
        let mut all_sectors = sectors();
        all_sectors.extend(unproven_sector.clone());
        let sector_arr = sectors_arr(&rt.store, all_sectors);

        // Add an unproven sector.
        let power = partition
            .add_sectors(&rt.store, false, &unproven_sector, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();
        let expected_power = power_for_sectors(SECTOR_SIZE, &unproven_sector);
        assert_eq!(expected_power, power);

        // fault sector 3, 4, 5 and 6
        let fault_set = make_bitfield(&[3, 4, 5, 6]);
        partition
            .record_faults(&rt.store, &sector_arr, &fault_set, 7, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        // mark 4 and 5 as a recoveries
        let recover_set = make_bitfield(&[4, 5]);
        partition.declare_faults_recovered(&sector_arr, SECTOR_SIZE, &recover_set).unwrap();

        // now terminate 1, 3, 5, and 7
        let terminations = make_bitfield(&[1, 3, 5, 7]);
        let termination_epoch = 3;
        let removed = partition
            .terminate_sectors(
                &Policy::default(),
                &rt.store,
                &sector_arr,
                termination_epoch,
                &terminations,
                SECTOR_SIZE,
                QUANT_SPEC,
            )
            .unwrap();

        let expected_active_power =
            power_for_sectors(SECTOR_SIZE, &select_sectors(&sectors(), &make_bitfield(&[1])));
        assert_eq!(expected_active_power, removed.active_power);
        let expected_faulty_power =
            power_for_sectors(SECTOR_SIZE, &select_sectors(&sectors(), &make_bitfield(&[3, 5])));
        assert_eq!(expected_faulty_power, removed.faulty_power);

        // expect partition state to no longer reflect power and pledge from terminated sectors and terminations to contain new sectors
        assert_partition_state(
            &rt.store,
            &partition,
            &sectors(),
            bitfield_from_slice(&[1, 2, 3, 4, 5, 6, 7]),
            bitfield_from_slice(&[4, 6]),
            bitfield_from_slice(&[4]),
            terminations,
            bitfield_from_slice(&[]),
        );

        // sectors should move to new expiration group
        assert_partition_expiration_queue(
            &rt.store,
            &partition,
            QUANT_SPEC,
            &[
                ExpectExpirationGroup { expiration: 5, sectors: bitfield_from_slice(&[2]) },
                ExpectExpirationGroup { expiration: 9, sectors: bitfield_from_slice(&[4, 6]) },
            ],
        );

        // sectors should be added to early termination bitfield queue
        let queue = BitFieldQueue::new(&rt.store, &partition.early_terminated, QUANT_SPEC).unwrap();

        BitFieldQueueExpectation::default().add(termination_epoch, &[1, 3, 5, 7]).equals(&queue);
    }

    #[test]
    fn terminate_non_existent_sectors() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        let terminations = make_bitfield(&[99]);
        let termination_epoch = 3;

        let res = partition.terminate_sectors(
            &Policy::default(),
            &rt.store,
            &sector_arr,
            termination_epoch,
            &terminations,
            SECTOR_SIZE,
            QUANT_SPEC,
        );

        let err = res.expect_err("expected error, but call succeeded");
        // XXX: This is not a good way to check for specific errors.
        //      See: https://github.com/filecoin-project/builtin-actors/issues/338
        assert!(err.to_string().contains("can only terminate live sectors"));
    }

    #[test]
    fn terminate_already_terminated_sector() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        let terminations = make_bitfield(&[1]);
        let termination_epoch = 3;

        // First termination works.
        let removed = partition
            .terminate_sectors(
                &Policy::default(),
                &rt.store,
                &sector_arr,
                termination_epoch,
                &terminations,
                SECTOR_SIZE,
                QUANT_SPEC,
            )
            .unwrap();
        let expected_active_power =
            power_for_sectors(SECTOR_SIZE, &select_sectors(&sectors(), &make_bitfield(&[1])));
        assert_eq!(expected_active_power, removed.active_power);
        assert_eq!(removed.faulty_power, PowerPair::zero());
        let count = removed.len();
        assert_eq!(1, count);

        // Second termination fails
        let res = partition.terminate_sectors(
            &Policy::default(),
            &rt.store,
            &sector_arr,
            termination_epoch,
            &terminations,
            SECTOR_SIZE,
            QUANT_SPEC,
        );

        let err = res.expect_err("expected error, but call succeeded");
        // XXX: This is not a good way to check for specific errors.
        //      See: https://github.com/filecoin-project/builtin-actors/issues/338
        assert!(err.to_string().contains("can only terminate live sectors"));
    }

    #[test]
    fn mark_terminated_sectors_as_faulty() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        let terminations = make_bitfield(&[1]);
        let termination_epoch = 3;

        // Termination works.
        partition
            .terminate_sectors(
                &Policy::default(),
                &rt.store,
                &sector_arr,
                termination_epoch,
                &terminations,
                SECTOR_SIZE,
                QUANT_SPEC,
            )
            .unwrap();

        // Fault declaration for terminated sectors fails.
        let (new_faults, power_delta, faulty_power) = partition
            .record_faults(&rt.store, &sector_arr, &terminations, 5, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();
        assert!(new_faults.is_empty());
        assert!(power_delta.is_zero());
        assert!(faulty_power.is_zero());
    }

    #[test]
    fn pop_expiring_sectors() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        // add one fault with an early termination
        let fault_set = make_bitfield(&[4]);
        let (_, _, _) = partition
            .record_faults(&rt.store, &sector_arr, &fault_set, 2, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        // pop first expiration set
        let expire_epoch = 5;
        let expset = partition.pop_expired_sectors(&rt.store, expire_epoch, QUANT_SPEC).unwrap();

        assert_bitfield_equals(&expset.on_time_sectors, &[1, 2]);
        assert_bitfield_equals(&expset.early_sectors, &[4]);
        assert_eq!(TokenAmount::from_atto(1000u64 + 1001), expset.on_time_pledge);

        // active power only contains power from non-faulty sectors
        assert_eq!(expset.active_power, power_for_sectors(SECTOR_SIZE, &sectors()[..2]));

        // faulty power comes from early termination
        assert_eq!(expset.faulty_power, power_for_sectors(SECTOR_SIZE, &sectors()[3..4]));

        // expect sectors to be moved to terminations
        assert_partition_state(
            &rt.store,
            &partition,
            &sectors(),
            bitfield_from_slice(&[1, 2, 3, 4, 5, 6]),
            BitField::new(),
            BitField::new(),
            bitfield_from_slice(&[1, 2, 4]),
            BitField::new(),
        );

        // sectors should move to new expiration group
        assert_partition_expiration_queue(
            &rt.store,
            &partition,
            QUANT_SPEC,
            &[
                ExpectExpirationGroup { expiration: 9, sectors: bitfield_from_slice(&[3]) },
                ExpectExpirationGroup { expiration: 13, sectors: bitfield_from_slice(&[5, 6]) },
            ],
        );

        // sectors should be added to early termination bitfield queue
        let queue = BitFieldQueue::new(&rt.store, &partition.early_terminated, QUANT_SPEC).unwrap();

        // only early termination appears in bitfield queue
        BitFieldQueueExpectation::default().add(expire_epoch, &[4]).equals(&queue);
    }

    #[test]
    fn pop_expiring_sectors_errors_if_a_recovery_exists() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        let _ = partition
            .record_faults(&rt.store, &sector_arr, &make_bitfield(&[5]), 2, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        // add a recovery
        partition.declare_faults_recovered(&sector_arr, SECTOR_SIZE, &make_bitfield(&[5])).unwrap();

        // pop first expiration set
        let expire_epoch = 5;
        let res = partition.pop_expired_sectors(&rt.store, expire_epoch, QUANT_SPEC);

        let err = res.expect_err("expected error, but call succeeded");
        // XXX: This is not a good way to check for specific errors.
        //      See: https://github.com/filecoin-project/builtin-actors/issues/338
        assert!(err.to_string().contains("unexpected recoveries while processing expirations"));
    }

    #[test]
    fn pop_expiring_sectors_errors_if_a_unproven_sectors_exist() {
        let (rt, mut partition) = setup_unproven();

        // pop first expiration set
        let expire_epoch = 5;
        let res = partition.pop_expired_sectors(&rt.store, expire_epoch, QUANT_SPEC);

        let err = res.expect_err("expected error, but call succeeded");
        // XXX: This is not a good way to check for specific errors.
        //      See: https://github.com/filecoin-project/builtin-actors/issues/338
        assert!(err
            .to_string()
            .contains("cannot pop expired sectors from a partition with unproven sectors"));
    }

    #[test]
    fn records_missing_post() {
        let (rt, mut partition) = setup_partition();

        let unproven_sector = vec![test_sector(13, 7, 55, 65, 1006)];
        let mut all_sectors = sectors();
        all_sectors.extend_from_slice(&unproven_sector);
        let sector_arr = sectors_arr(&rt.store, sectors());

        // Add an unproven sector.
        let power = partition
            .add_sectors(&rt.store, false, &unproven_sector, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();
        let expected_power = power_for_sectors(SECTOR_SIZE, &unproven_sector);
        assert_eq!(expected_power, power);

        // make 4, 5 and 6 faulty
        let fault_set = make_bitfield(&[4, 5, 6]);
        partition
            .record_faults(&rt.store, &sector_arr, &fault_set, 7, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        // add 4 and 5 as recoveries
        let recover_set = make_bitfield(&[4, 5]);
        partition.declare_faults_recovered(&sector_arr, SECTOR_SIZE, &recover_set).unwrap();

        // record entire partition as faulted
        let (power_delta, penalized_power, _) =
            partition.record_missed_post(&rt.store, 6, QUANT_SPEC).unwrap();

        // 6 has always been faulty, so we shouldn't be penalized for it (except ongoing).
        let expected_penalized_power = power_for_sectors(SECTOR_SIZE, &all_sectors)
            - power_for_sectors(SECTOR_SIZE, &all_sectors[5..6]);
        assert_eq!(expected_penalized_power, penalized_power);

        // We should lose power for sectors 1-3.
        let expected_power_delta = -power_for_sectors(SECTOR_SIZE, &all_sectors[..3]);
        assert_eq!(expected_power_delta, power_delta);

        // everything is now faulty
        assert_partition_state(
            &rt.store,
            &partition,
            &all_sectors,
            bitfield_from_slice(&[1, 2, 3, 4, 5, 6, 7]),
            bitfield_from_slice(&[1, 2, 3, 4, 5, 6, 7]),
            BitField::new(),
            BitField::new(),
            BitField::new(),
        );

        // everything not in first expiration group is now in second because fault expiration quantized to 9
        assert_partition_expiration_queue(
            &rt.store,
            &partition,
            QUANT_SPEC,
            &[
                ExpectExpirationGroup { expiration: 5, sectors: bitfield_from_slice(&[1, 2]) },
                ExpectExpirationGroup {
                    expiration: 9,
                    sectors: bitfield_from_slice(&[3, 4, 5, 6, 7]),
                },
            ],
        );
    }

    #[test]
    fn pops_early_terminations() {
        let (rt, mut partition) = setup_partition();
        let sector_arr = sectors_arr(&rt.store, sectors());

        // fault sector 3, 4, 5 and 6
        let fault_set = make_bitfield(&[3, 4, 5, 6]);
        partition
            .record_faults(&rt.store, &sector_arr, &fault_set, 7, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();

        // mark 4 and 5 as a recoveries
        let recover_set = make_bitfield(&[4, 5]);
        partition.declare_faults_recovered(&sector_arr, SECTOR_SIZE, &recover_set).unwrap();

        // now terminate 1, 3 and 5
        let terminations = make_bitfield(&[1, 3, 5]);
        let termination_epoch = 3;
        partition
            .terminate_sectors(
                &Policy::default(),
                &rt.store,
                &sector_arr,
                termination_epoch,
                &terminations,
                SECTOR_SIZE,
                QUANT_SPEC,
            )
            .unwrap();

        // pop first termination
        let (result, has_more) = partition.pop_early_terminations(&rt.store, 1).unwrap();

        // expect first sector to be in early terminations
        assert_bitfield_equals(&result.sectors[&termination_epoch], &[1]);

        // expect more results
        assert!(has_more);

        // expect terminations to still contain 3 and 5
        let queue = BitFieldQueue::new(&rt.store, &partition.early_terminated, QUANT_SPEC).unwrap();

        // only early termination appears in bitfield queue
        BitFieldQueueExpectation::default().add(termination_epoch, &[3, 5]).equals(&queue);

        // pop the rest
        let (result, has_more) = partition.pop_early_terminations(&rt.store, 5).unwrap();

        // expect 3 and 5
        assert_bitfield_equals(&result.sectors[&termination_epoch], &[3, 5]);

        // expect no more results
        assert!(!has_more);

        // expect early terminations to be empty
        let queue = BitFieldQueue::new(&rt.store, &partition.early_terminated, QUANT_SPEC).unwrap();
        BitFieldQueueExpectation::default().equals(&queue);
    }

    #[test]
    fn test_max_sectors() {
        let (_, rt) = setup();
        let mut partition = Partition::new(&rt.store).unwrap();

        let proof_type = RegisteredSealProof::StackedDRG32GiBV1P1;
        let _sector_size = proof_type.sector_size().unwrap();
        let partition_sectors = proof_type.window_post_partitions_sector().unwrap();

        let mut many_sectors = vec![SectorOnChainInfo::default(); partition_sectors as usize];
        let mut ids = vec![0u64; partition_sectors as usize];
        for (i, info) in many_sectors.iter_mut().enumerate() {
            let id = (i as u64 + 1) << 50;
            ids[i as usize] = id;
            *info = test_sector(i as i64 + 1, id, 50, 60, 1000);
        }
        let sector_numbers = bitfield_from_slice(&ids);

        let power = partition
            .add_sectors(&rt.store, false, &many_sectors, SECTOR_SIZE, QUANT_SPEC)
            .unwrap();
        let expected_power = power_for_sectors(SECTOR_SIZE, &many_sectors);
        assert_eq!(expected_power, power);

        assert_partition_state(
            &rt.store,
            &partition,
            &many_sectors,
            sector_numbers.clone(),
            BitField::new(),
            BitField::new(),
            BitField::new(),
            sector_numbers.clone(),
        );

        // Make sure we can still encode and decode.
        let buf = RawBytes::serialize(&partition).expect("failed to marshal partition");
        let new_partition: Partition = buf.deserialize().unwrap();
        assert_partition_state(
            &rt.store,
            &new_partition,
            &many_sectors,
            sector_numbers.clone(),
            BitField::new(),
            BitField::new(),
            BitField::new(),
            sector_numbers,
        );
    }
}
