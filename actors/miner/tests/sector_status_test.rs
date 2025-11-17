use fil_actors_runtime::test_utils::{MockRuntime, expect_abort};
use fvm_ipld_bitfield::BitField;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use num_traits::Zero;

use fil_actor_miner::SectorStatusCode;

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
fn generate_and_validate_active_sector_status() {
    let (mut h, rt) = setup();

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);
    let sector_number = sectors[0].sector_number;

    let (status, aux_data) = h.generate_sector_status_info(&rt, sector_number).unwrap();

    assert_eq!(status, SectorStatusCode::Active);

    let is_valid = h.validate_sector_status_info(&rt, sector_number, status, aux_data).unwrap();
    assert!(is_valid);

    h.check_state(&rt);
}

#[test]
fn generate_and_validate_terminated_sector_status() {
    let (mut h, rt) = setup();

    let sectors = h.commit_and_prove_sectors(&rt, 3, DEFAULT_SECTOR_EXPIRATION, vec![], true);
    let sector_number = sectors[0].sector_number;

    // necessary for terminate_sectors to do correct math
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());

    let mut sectors = BitField::new();
    sectors.set(sector_number);
    let expected_fee = TokenAmount::from_atto(3238952636718750000u128); // 3.23895263671875FIL
    let (_, _) = h.terminate_sectors(&rt, &sectors, expected_fee);

    let (status, aux_data) = h.generate_sector_status_info(&rt, sector_number).unwrap();

    assert_eq!(status, SectorStatusCode::Terminated);

    let is_valid = h.validate_sector_status_info(&rt, sector_number, status, aux_data).unwrap();
    assert!(is_valid);

    h.check_state(&rt);
}

#[test]
fn validate_fails_with_wrong_status() {
    let (mut h, rt) = setup();

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);
    let sector_number = sectors[0].sector_number;

    let (_status, aux_data) = h.generate_sector_status_info(&rt, sector_number).unwrap();

    // try to validate with wrong status (should be Active but we say Terminated)
    let is_valid = h
        .validate_sector_status_info(&rt, sector_number, SectorStatusCode::Terminated, aux_data)
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
    let result = h.validate_sector_status_info(
        &rt,
        sector_number,
        SectorStatusCode::Active,
        invalid_aux_data,
    );
    expect_abort(ExitCode::USR_ILLEGAL_ARGUMENT, result);

    h.check_state(&rt);
}

#[test]
fn generate_fails_for_nonexistent_sector() {
    let (mut h, rt) = setup();
    h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);

    // try to generate status for non-existent sector
    let result = h.generate_sector_status_info(&rt, 999);
    expect_abort(ExitCode::USR_ILLEGAL_STATE, result);

    h.check_state(&rt);
}

#[test]
fn validate_fails_with_nonexistent_sector_in_partition() {
    let (mut h, rt) = setup();

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![vec![10]], true);
    let sector_number = sectors[0].sector_number;

    let (status, aux_data) = h.generate_sector_status_info(&rt, sector_number).unwrap();

    // try to validate different sector number with same aux_data
    let is_valid = h.validate_sector_status_info(&rt, 999, status, aux_data).unwrap();
    assert!(!is_valid);

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

    // Generate status info for sector 1
    let (status1, aux_data1) = h.generate_sector_status_info(&rt, sector1).unwrap();

    // Apply rewards for termination fee calculation
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());
    // Terminate sector 2 to create different status
    let mut terminate_bf = BitField::new();
    terminate_bf.set(sector2);
    let expected_fee = TokenAmount::from_atto(3238952636718750000u128);
    let (_, _) = h.terminate_sectors(&rt, &terminate_bf, expected_fee);

    // Test 9a: Try to validate sector 2 with sector 1's status
    let is_valid_9a =
        h.validate_sector_status_info(&rt, sector2, status1, aux_data1.clone()).unwrap();
    assert!(!is_valid_9a);

    // Test 9b: Try to validate sector 1 with sector 2's status
    let (status2, _aux_data2) = h.generate_sector_status_info(&rt, sector2).unwrap();
    let is_valid_9b =
        h.validate_sector_status_info(&rt, sector1, status2, aux_data1.clone()).unwrap();
    assert!(!is_valid_9b);

    h.check_state(&rt);
}
