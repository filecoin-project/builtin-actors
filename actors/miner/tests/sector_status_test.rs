use fil_actor_miner::{
    NO_DEADLINE, NO_PARTITION, SectorLocation, SectorStatusCode, State,
    pledge_penalty_for_continued_fault, pledge_penalty_for_termination, qa_power_for_sector,
};
use fil_actors_runtime::runtime::{Runtime, RuntimePolicy};
use fil_actors_runtime::test_utils::{MockRuntime, expect_abort};
use fvm_ipld_bitfield::BitField;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use num_traits::Zero;

mod util;
use util::*;

fn setup() -> (ActorHarness, MockRuntime) {
    let period_offset = 100;

    let h = ActorHarness::new(period_offset);
    let rt = h.new_runtime();
    h.construct_and_verify(&rt);
    rt.balance.replace(BIG_BALANCE.clone());

    (h, rt)
}

#[test]
fn generate_sector_location_live_sector() {
    let (mut h, rt) = setup();

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);
    let sector_number = sectors[0].sector_number;

    let (status, aux_data) = h.generate_sector_location(&rt, sector_number).unwrap();
    assert_eq!(status, SectorStatusCode::Active);

    let is_valid =
        h.validate_sector_status(&rt, sector_number, status, aux_data).unwrap();
    assert!(is_valid);

    h.check_state(&rt);
}

#[test]
fn generate_sector_location_terminated_not_compacted() {
    let (mut h, rt) = setup();

    let sectors = h.commit_and_prove_sectors(&rt, 3, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let sector_number = sectors[0].sector_number;

    // necessary for terminate_sectors to do correct math
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());

    let mut sectors = BitField::new();
    sectors.set(sector_number);
    let expected_fee = TokenAmount::from_atto(3238952636718750000u128); // 3.23895263671875FIL
    let (_, _) = h.terminate_sectors(&rt, &sectors, expected_fee);

    let (status, aux_data) = h.generate_sector_location(&rt, sector_number).unwrap();
    assert_eq!(status, SectorStatusCode::Dead);

    let is_valid =
        h.validate_sector_status(&rt, sector_number, status, aux_data).unwrap();
    assert!(is_valid);

    h.check_state(&rt);
}

#[test]
fn validate_live_sector_as_dead_or_faulty_returns_false() {
    let (mut h, rt) = setup();

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);
    let sector_number = sectors[0].sector_number;

    let (_status, aux_data) = h.generate_sector_location(&rt, sector_number).unwrap();

    let is_valid = h
        .validate_sector_status(&rt, sector_number, SectorStatusCode::Dead, aux_data.clone())
        .unwrap();
    assert!(!is_valid);

    let is_valid = h
        .validate_sector_status(&rt, sector_number, SectorStatusCode::Faulty, aux_data)
        .unwrap();
    assert!(!is_valid);

    h.check_state(&rt);
}

#[test]
fn validate_fails_with_invalid_aux_data() {
    let (mut h, rt) = setup();

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);
    let sector_number = sectors[0].sector_number;

    // try to validate with invalid aux_data
    let invalid_aux_data = vec![1, 2, 3, 4]; // Invalid CBOR
    let result = h.validate_sector_status(
        &rt,
        sector_number,
        SectorStatusCode::Active,
        invalid_aux_data,
    );
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);

    h.check_state(&rt);
}

#[test]
fn generate_sector_location_nonexistent_sector() {
    let (mut h, rt) = setup();
    h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);

    // non-existent sector should return NO_DEADLINE/NO_PARTITION, not error
    let (status, aux_data) = h.generate_sector_location(&rt, 999).unwrap();
    assert_eq!(status, SectorStatusCode::Dead);

    let location: SectorLocation = fvm_ipld_encoding::from_slice(&aux_data).unwrap();
    assert_eq!(location.deadline, NO_DEADLINE);
    assert_eq!(location.partition, NO_PARTITION);

    h.check_state(&rt);
}

#[test]
fn validate_fails_with_sector_not_found_at_location() {
    let (mut h, rt) = setup();

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);
    let sector_number = sectors[0].sector_number;

    let (_status, aux_data) = h.generate_sector_location(&rt, sector_number).unwrap();

    // try to validate different sector number with same aux_data — should error
    let result = h.validate_sector_status(&rt, 999, SectorStatusCode::Active, aux_data);
    expect_abort(ExitCode::USR_NOT_FOUND, result);

    h.check_state(&rt);
}

#[test]
fn validate_detects_swapped_sector_number() {
    let (mut h, rt) = setup();

    // Commit and prove two sectors
    let sectors = h.commit_and_prove_sectors(
        &rt,
        2,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![11]],
        true,
    );
    let sector1 = sectors[0].sector_number;
    let sector2 = sectors[1].sector_number;

    // Generate location for sector 1
    let (status1, aux_data1) = h.generate_sector_location(&rt, sector1).unwrap();

    // Apply rewards for termination fee calculation
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());
    // Terminate sector 2 to create different status
    let mut terminate_bf = BitField::new();
    terminate_bf.set(sector2);
    let expected_fee = TokenAmount::from_atto(3238952636718750000u128);
    let (_, _) = h.terminate_sectors(&rt, &terminate_bf, expected_fee);

    // Test 9a: Try to validate sector 2 with sector 1's status
    let is_valid_9a =
        h.validate_sector_status(&rt, sector2, status1, aux_data1.clone()).unwrap();
    assert!(!is_valid_9a);

    // Test 9b: Try to validate sector 1 with sector 2's status
    let (status2, _aux_data2) = h.generate_sector_location(&rt, sector2).unwrap();
    let is_valid_9b =
        h.validate_sector_status(&rt, sector1, status2, aux_data1.clone()).unwrap();
    assert!(!is_valid_9b);

    h.check_state(&rt);
}

#[test]
fn generate_sector_location_compacted_terminated() {
    let (mut h, rt) = setup();
    rt.set_epoch(200);

    let sectors_info = h.commit_and_prove_sectors(
        &rt,
        4,
        DEFAULT_SECTOR_EXPIRATION,
        vec![vec![10], vec![20], vec![30], vec![40]],
        true,
    );
    h.advance_and_submit_posts(&rt, &sectors_info);

    let sector_number = sectors_info[0].sector_number;

    // Terminate a sector
    let epoch = *rt.epoch.borrow();
    rt.set_epoch(epoch + 100);
    // necessary for terminate_sectors to do correct math
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());

    let sector_size = sectors_info[0].seal_proof.sector_size().unwrap();
    let sector_age = *rt.epoch.borrow() - sectors_info[0].activation;
    let fault_fee = pledge_penalty_for_continued_fault(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &qa_power_for_sector(sector_size, &sectors_info[0]),
    );
    let expected_fee =
        pledge_penalty_for_termination(&sectors_info[0].initial_pledge, sector_age, &fault_fee);
    h.terminate_sectors(&rt, &bitfield_from_slice(&[sector_number]), expected_fee);

    // Wait WPoStProofChallengePeriod then compact
    let target_epoch = *rt.epoch.borrow() + rt.policy().wpost_dispute_window;
    h.advance_to_epoch_with_cron(&rt, target_epoch);
    h.compact_partitions(&rt, 0, bitfield_from_slice(&[0])).unwrap();

    // Sector should now be fully removed from state
    let state: State = rt.get_state();
    assert!(state.get_sector(rt.store(), sector_number).unwrap().is_none());

    // generate_sector_location should return NO_DEADLINE/NO_PARTITION
    let (status, aux_data) = h.generate_sector_location(&rt, sector_number).unwrap();
    assert_eq!(status, SectorStatusCode::Dead);

    let location: SectorLocation = fvm_ipld_encoding::from_slice(&aux_data).unwrap();
    assert_eq!(location.deadline, NO_DEADLINE);
    assert_eq!(location.partition, NO_PARTITION);

    h.check_state(&rt);
}

#[test]
fn validate_dead_sector_no_location() {
    let (mut h, rt) = setup();
    h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);

    // Sector 999 was never committed — generate returns NO location
    let (status, aux_data) = h.generate_sector_location(&rt, 999).unwrap();
    assert_eq!(status, SectorStatusCode::Dead);

    let is_valid =
        h.validate_sector_status(&rt, 999, SectorStatusCode::Dead, aux_data).unwrap();
    assert!(is_valid);

    h.check_state(&rt);
}

#[test]
fn validate_faulty_sector() {
    let (mut h, rt) = setup();

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);
    let sector_number = sectors[0].sector_number;

    // Must advance and submit posts before declaring faults
    h.advance_and_submit_posts(&rt, &sectors);
    h.advance_deadline(&rt, CronConfig::empty());
    h.declare_faults(&rt, &sectors);

    let (status, aux_data) = h.generate_sector_location(&rt, sector_number).unwrap();
    assert_eq!(status, SectorStatusCode::Faulty);

    let is_valid = h
        .validate_sector_status(&rt, sector_number, SectorStatusCode::Faulty, aux_data)
        .unwrap();
    assert!(is_valid);

    h.check_state(&rt);
}

#[test]
fn validate_faulty_sector_as_live_or_dead_returns_false() {
    let (mut h, rt) = setup();

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);
    let sector_number = sectors[0].sector_number;

    h.advance_and_submit_posts(&rt, &sectors);
    h.advance_deadline(&rt, CronConfig::empty());
    h.declare_faults(&rt, &sectors);

    let (_status, aux_data) = h.generate_sector_location(&rt, sector_number).unwrap();

    let is_valid =
        h.validate_sector_status(&rt, sector_number, SectorStatusCode::Active, aux_data.clone()).unwrap();
    assert!(!is_valid);

    let is_valid =
        h.validate_sector_status(&rt, sector_number, SectorStatusCode::Dead, aux_data).unwrap();
    assert!(!is_valid);

    h.check_state(&rt);
}

#[test]
fn validate_dead_sector_as_live_or_faulty_returns_false() {
    let (mut h, rt) = setup();

    let sectors = h.commit_and_prove_sectors(&rt, 3, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let sector_number = sectors[0].sector_number;

    // necessary for terminate_sectors to do correct math
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());

    let mut sectors = BitField::new();
    sectors.set(sector_number);
    let expected_fee = TokenAmount::from_atto(3238952636718750000u128); // 3.23895263671875FIL
    let (_, _) = h.terminate_sectors(&rt, &sectors, expected_fee);

    let (_status, aux_data) = h.generate_sector_location(&rt, sector_number).unwrap();

    let is_valid =
        h.validate_sector_status(&rt, sector_number, SectorStatusCode::Active, aux_data.clone()).unwrap();
    assert!(!is_valid);

    let is_valid = h
        .validate_sector_status(&rt, sector_number, SectorStatusCode::Faulty, aux_data)
        .unwrap();
    assert!(!is_valid);

    h.check_state(&rt);
}

#[test]
fn validate_fails_with_mixed_no_location() {
    let (mut h, rt) = setup();
    h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);

    // NO_DEADLINE but valid partition — invalid combination
    let bad_location = SectorLocation { deadline: NO_DEADLINE, partition: 0 };
    let aux_data = fvm_ipld_encoding::to_vec(&bad_location).unwrap();
    let result = h.validate_sector_status(&rt, 0, SectorStatusCode::Dead, aux_data);
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);

    // Valid deadline but NO_PARTITION — invalid combination
    let bad_location = SectorLocation { deadline: 0, partition: NO_PARTITION };
    let aux_data = fvm_ipld_encoding::to_vec(&bad_location).unwrap();
    let result = h.validate_sector_status(&rt, 0, SectorStatusCode::Dead, aux_data);
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);

    h.check_state(&rt);
}

#[test]
fn validate_live_or_faulty_at_no_location_returns_false() {
    let (mut h, rt) = setup();

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);
    let sector_number = sectors[0].sector_number;

    let no_location = SectorLocation { deadline: NO_DEADLINE, partition: NO_PARTITION };
    let aux_data = fvm_ipld_encoding::to_vec(&no_location).unwrap();

    let is_valid = h
        .validate_sector_status(&rt, sector_number, SectorStatusCode::Active, aux_data.clone())
        .unwrap();
    assert!(!is_valid);

    let is_valid = h
        .validate_sector_status(&rt, sector_number, SectorStatusCode::Faulty, aux_data)
        .unwrap();
    assert!(!is_valid);

    h.check_state(&rt);
}

// ---- GetNominalSectorExpiration tests ----

#[test]
fn get_nominal_sector_expiration_returns_correct() {
    let (mut h, rt) = setup();

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);
    let sector_number = sectors[0].sector_number;

    let expiration = h.get_nominal_sector_expiration(&rt, sector_number).unwrap();
    assert_eq!(expiration, sectors[0].expiration);

    h.check_state(&rt);
}

#[test]
fn get_nominal_sector_expiration_fails_not_in_amt() {
    let (mut h, rt) = setup();
    h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);

    let result = h.get_nominal_sector_expiration(&rt, 999);
    expect_abort(ExitCode::USR_NOT_FOUND, result);

    h.check_state(&rt);
}

#[test]
fn get_nominal_sector_expiration_after_early_termination() {
    let (mut h, rt) = setup();

    let sectors = h.commit_and_prove_sectors(&rt, 3, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let sector_number = sectors[0].sector_number;
    let original_expiration = sectors[0].expiration;

    // necessary for terminate_sectors to do correct math
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());

    let mut sectors = BitField::new();
    sectors.set(sector_number);
    let expected_fee = TokenAmount::from_atto(3238952636718750000u128); // 3.23895263671875FIL
    let (_, _) = h.terminate_sectors(&rt, &sectors, expected_fee);

    // Sector is terminated but still in AMT — returns the nominal (stale) value
    let expiration = h.get_nominal_sector_expiration(&rt, sector_number).unwrap();
    assert_eq!(expiration, original_expiration);

    h.check_state(&rt);
}
