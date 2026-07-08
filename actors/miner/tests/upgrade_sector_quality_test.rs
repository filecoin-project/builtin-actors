use fvm_ipld_bitfield::BitField;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::SectorNumber;

use fil_actor_miner::{
    Actor, Deadline, Method, SectorOnChainInfo, SectorOnChainInfoFlags, Sectors, State,
    UpgradeSectorQualityParams, qa_power_for_sector, qa_power_max,
};
use fil_actors_runtime::test_utils::{ACCOUNT_ACTOR_CODE_ID, MockRuntime};
use fil_actors_runtime::{BatchReturn, EPOCHS_IN_DAY, STORAGE_POWER_ACTOR_ADDR};
use fvm_ipld_encoding::ipld_block::IpldBlock;
use num_traits::Zero;

use fil_actor_miner::ext::power::UPDATE_CLAIMED_POWER_METHOD;
use fil_actor_power::{Method as PowerMethod, UpdateClaimedPowerParams};

mod util;
use util::*;

const PERIOD_OFFSET: ChainEpoch = 100;
const DEFAULT_SECTOR_EXPIRATION_DAYS: ChainEpoch = 220;
const FIRST_SECTOR_NUMBER: SectorNumber = 100;

fn setup() -> (ActorHarness, MockRuntime) {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&rt);
    (h, rt)
}

/// Create sectors and submit their first Window PoSt so they are active.
fn setup_sectors(count: usize) -> (ActorHarness, MockRuntime, Vec<SectorOnChainInfo>) {
    let (h, rt) = setup();
    let sector_expiry = *rt.epoch.borrow() + DEFAULT_SECTOR_EXPIRATION_DAYS * EPOCHS_IN_DAY;
    let sectors = onboard_empty_sectors(&rt, &h, sector_expiry, FIRST_SECTOR_NUMBER, count);
    (h, rt, sectors)
}

/// Downgrades a sector by clearing FULL_QA_POWER (and optionally daily_fee) in sector AMT,
/// and uses partition.replace_sectors to keep partition/deadline power consistent.
fn downgrade_sector_in_state(
    h: &ActorHarness,
    rt: &MockRuntime,
    sno: SectorNumber,
    zero_daily_fee: bool,
) {
    let old_sector = h.get_sector(rt, sno);

    let mut modified = old_sector.clone();
    modified.flags.remove(SectorOnChainInfoFlags::FULL_QA_POWER);
    if zero_daily_fee {
        modified.daily_fee = TokenAmount::zero();
    }

    let mut st: State = rt.get_state();
    let (dl_idx, part_idx) = st.find_sector(&rt.store, sno).unwrap();
    let quant = st.quant_spec_for_deadline(&rt.policy, dl_idx);

    // Update sectors AMT.
    {
        let mut sectors_amt = Sectors::load(&rt.store, &st.sectors).unwrap();
        sectors_amt.store(vec![modified.clone()]).unwrap();
        st.sectors = sectors_amt.amt.flush().unwrap();
    }

    // Update partition via replace_sectors so power/pledge/fee accounting stays consistent.
    let mut deadlines = st.load_deadlines(&rt.store).unwrap();
    let mut deadline: Deadline = deadlines.load_deadline(&rt.store, dl_idx).unwrap();
    let mut partitions = deadline.partitions_amt(&rt.store).unwrap();
    let mut partition = partitions.get(part_idx).unwrap().unwrap().clone();

    let (power_delta, pledge_delta, fee_delta) = partition
        .replace_sectors(&rt.store, &[old_sector], &[modified], h.sector_size, quant)
        .unwrap();

    partitions.set(part_idx, partition).unwrap();
    deadline.partitions = partitions.flush().unwrap();
    deadline.live_power += &power_delta;
    deadline.daily_fee += &fee_delta;

    deadlines.update_deadline(&rt.policy, &rt.store, dl_idx, &deadline).unwrap();
    st.save_deadlines(&rt.store, deadlines).unwrap();

    // Adjust state-level initial pledge.
    st.initial_pledge += &pledge_delta;

    rt.replace_state(&st);
}

/// Helper to call UpgradeSectorQuality expecting that no valid sectors pass validation,
/// so network info queries will NOT happen.
fn upgrade_sector_quality_no_valid(
    h: &ActorHarness,
    rt: &MockRuntime,
    sectors: &BitField,
) -> BatchReturn {
    let params = UpgradeSectorQualityParams { sectors: sectors.clone() };

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    rt.expect_validate_caller_addr(h.caller_addrs());

    // No valid sectors pass validation, so no network info query or power/pledge updates happen.
    let result = rt.call::<Actor>(
        Method::UpgradeSectorQuality as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    );

    let batch_return: BatchReturn = result.unwrap().unwrap().deserialize().unwrap();
    rt.verify();
    batch_return
}

/// New sectors get FULL_QA_POWER automatically. Trying to upgrade them should fail
/// with USR_ILLEGAL_ARGUMENT for each sector.
#[test]
fn upgrade_already_full_qa_rejected() {
    let (h, rt, sectors) = setup_sectors(1);
    let sno = sectors[0].sector_number;

    // Verify the sector already has FULL_QA_POWER flag.
    let sector = h.get_sector(&rt, sno);
    assert!(
        sector.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER),
        "new sector should have FULL_QA_POWER flag"
    );

    let bf = BitField::try_from_bits([sno]).unwrap();
    let result = upgrade_sector_quality_no_valid(&h, &rt, &bf);

    assert_eq!(1, result.size());
    assert_eq!(0, result.success_count as usize);
    assert_eq!(1, result.fail_codes.len());
    assert_eq!(ExitCode::USR_ILLEGAL_ARGUMENT, result.fail_codes[0].code);

    h.check_state(&rt);
}

/// Trying to upgrade a nonexistent sector should fail with USR_NOT_FOUND.
#[test]
fn upgrade_nonexistent_sector_rejected() {
    let (h, rt) = setup();

    let bf = BitField::try_from_bits([99999u64]).unwrap();
    let result = upgrade_sector_quality_no_valid(&h, &rt, &bf);

    assert_eq!(1, result.size());
    assert_eq!(0, result.success_count as usize);
    assert_eq!(1, result.fail_codes.len());
    assert_eq!(ExitCode::USR_NOT_FOUND, result.fail_codes[0].code);

    h.check_state(&rt);
}

/// Calling with an empty BitField should return an empty BatchReturn.
#[test]
fn upgrade_empty_bitfield() {
    let (h, rt) = setup();

    let params = UpgradeSectorQualityParams { sectors: BitField::new() };

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    rt.expect_validate_caller_addr(h.caller_addrs());

    let result = rt
        .call::<Actor>(
            Method::UpgradeSectorQuality as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap();

    let batch_return: BatchReturn = result.unwrap().deserialize().unwrap();
    rt.verify();

    assert_eq!(0, batch_return.size());
    assert_eq!(0, batch_return.success_count as usize);

    h.check_state(&rt);
}

/// Upgrading a mix of an already-FULL_QA sector and a nonexistent sector:
/// both should fail in the BatchReturn with appropriate error codes.
#[test]
fn upgrade_mixed_valid_invalid() {
    let (h, rt, sectors) = setup_sectors(1);
    let existing_sno = sectors[0].sector_number;
    let nonexistent_sno = 99999u64;

    let bf = BitField::try_from_bits([existing_sno, nonexistent_sno]).unwrap();
    let result = upgrade_sector_quality_no_valid(&h, &rt, &bf);

    assert_eq!(2, result.size());
    assert_eq!(0, result.success_count as usize);
    assert_eq!(2, result.fail_codes.len());

    // BitField iterates in sorted order: index 0 = existing_sno (100), index 1 = 99999.
    // Index 0 = existing_sno (100): already has FULL_QA_POWER
    assert_eq!(0, result.fail_codes[0].idx);
    assert_eq!(ExitCode::USR_ILLEGAL_ARGUMENT, result.fail_codes[0].code);
    // Index 1 = nonexistent_sno (99999): not found
    assert_eq!(1, result.fail_codes[1].idx);
    assert_eq!(ExitCode::USR_NOT_FOUND, result.fail_codes[1].code);

    h.check_state(&rt);
}

/// Test that upgrading multiple already-upgraded sectors correctly reports each failure.
#[test]
fn upgrade_multiple_already_full_qa() {
    let (h, rt, sectors) = setup_sectors(3);
    let snos: Vec<u64> = sectors.iter().map(|s| s.sector_number).collect();

    let bf = BitField::try_from_bits(snos.iter().copied()).unwrap();
    let result = upgrade_sector_quality_no_valid(&h, &rt, &bf);

    assert_eq!(3, result.size());
    assert_eq!(0, result.success_count as usize);
    assert_eq!(3, result.fail_codes.len());
    for fc in &result.fail_codes {
        assert_eq!(ExitCode::USR_ILLEGAL_ARGUMENT, fc.code);
    }

    h.check_state(&rt);
}

/// Test upgrading a legacy sector (without FULL_QA_POWER) to 10x.
/// We create a sector normally (which gets FULL_QA_POWER), then use downgrade_sector_in_state
/// to properly clear the flag and adjust partition/deadline power, simulating a legacy sector.
#[test]
fn upgrade_legacy_sector_to_full_qa() {
    let (h, rt, sectors) = setup_sectors(1);
    let sno = sectors[0].sector_number;

    // Remember the original sector (with FULL_QA_POWER) for comparison.
    let sector_original = h.get_sector(&rt, sno);
    assert!(sector_original.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));

    // Downgrade the sector to simulate a legacy sector without FULL_QA_POWER.
    downgrade_sector_in_state(&h, &rt, sno, false);

    let sector_legacy = h.get_sector(&rt, sno);
    assert!(!sector_legacy.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));

    // Compute expected deltas.
    let new_qa_power = qa_power_max(h.sector_size);
    let old_qa_power = qa_power_for_sector(h.sector_size, &sector_legacy);
    let qa_power_delta = &new_qa_power - &old_qa_power;

    let new_pledge = h.initial_pledge_for_power(&rt, &new_qa_power);
    let pledge_delta = if new_pledge > sector_legacy.initial_pledge {
        &new_pledge - &sector_legacy.initial_pledge
    } else {
        TokenAmount::zero()
    };

    // Call UpgradeSectorQuality.
    let params = UpgradeSectorQualityParams { sectors: BitField::try_from_bits([sno]).unwrap() };

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    rt.expect_validate_caller_addr(h.caller_addrs());
    h.expect_query_network_info(&rt);

    // Expect power update: raw stays the same, QA increases.
    rt.expect_send_simple(
        STORAGE_POWER_ACTOR_ADDR,
        UPDATE_CLAIMED_POWER_METHOD,
        IpldBlock::serialize_cbor(&UpdateClaimedPowerParams {
            raw_byte_delta: BigInt::zero(),
            quality_adjusted_delta: qa_power_delta.clone(),
        })
        .unwrap(),
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );

    // Expect pledge update.
    if pledge_delta.is_positive() {
        rt.expect_send_simple(
            STORAGE_POWER_ACTOR_ADDR,
            PowerMethod::UpdatePledgeTotal as u64,
            IpldBlock::serialize_cbor(&pledge_delta).unwrap(),
            TokenAmount::zero(),
            None,
            ExitCode::OK,
        );
    }

    let result = rt
        .call::<Actor>(
            Method::UpgradeSectorQuality as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap();
    let batch_return: BatchReturn = result.unwrap().deserialize().unwrap();
    rt.verify();

    // Verify success.
    assert_eq!(1, batch_return.size());
    assert_eq!(1, batch_return.success_count as usize);
    assert!(batch_return.fail_codes.is_empty());

    // Verify the sector now has FULL_QA_POWER flag.
    let sector_after = h.get_sector(&rt, sno);
    assert!(
        sector_after.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER),
        "sector should have FULL_QA_POWER flag after upgrade"
    );

    // Verify pledge did not decrease.
    assert!(
        sector_after.initial_pledge >= sector_legacy.initial_pledge,
        "pledge should not decrease after upgrade"
    );

    // Verify QA power for the sector is now qa_power_max (10x).
    let sector_qa = qa_power_for_sector(h.sector_size, &sector_after);
    assert_eq!(qa_power_max(h.sector_size), sector_qa);

    h.check_state(&rt);
}

/// Test that upgrading a legacy sector adjusts daily fee from zero.
/// Pre-FIP-0100 sectors had zero daily fee; upgrading should set a non-zero fee.
#[test]
fn upgrade_legacy_sector_sets_daily_fee() {
    let (h, rt, sectors) = setup_sectors(1);
    let sno = sectors[0].sector_number;

    // Downgrade sector: clear FULL_QA_POWER and zero out daily_fee to simulate pre-FIP-0100.
    downgrade_sector_in_state(&h, &rt, sno, true);

    let sector_legacy = h.get_sector(&rt, sno);
    assert!(!sector_legacy.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    assert!(sector_legacy.daily_fee.is_zero());

    // Compute expected deltas.
    let new_qa_power = qa_power_max(h.sector_size);
    let old_qa_power = qa_power_for_sector(h.sector_size, &sector_legacy);
    let qa_power_delta = &new_qa_power - &old_qa_power;

    let new_pledge = h.initial_pledge_for_power(&rt, &new_qa_power);
    let pledge_delta = if new_pledge > sector_legacy.initial_pledge {
        &new_pledge - &sector_legacy.initial_pledge
    } else {
        TokenAmount::zero()
    };

    // Call UpgradeSectorQuality.
    let params = UpgradeSectorQualityParams { sectors: BitField::try_from_bits([sno]).unwrap() };

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    rt.expect_validate_caller_addr(h.caller_addrs());
    h.expect_query_network_info(&rt);

    rt.expect_send_simple(
        STORAGE_POWER_ACTOR_ADDR,
        UPDATE_CLAIMED_POWER_METHOD,
        IpldBlock::serialize_cbor(&UpdateClaimedPowerParams {
            raw_byte_delta: BigInt::zero(),
            quality_adjusted_delta: qa_power_delta.clone(),
        })
        .unwrap(),
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );

    if pledge_delta.is_positive() {
        rt.expect_send_simple(
            STORAGE_POWER_ACTOR_ADDR,
            PowerMethod::UpdatePledgeTotal as u64,
            IpldBlock::serialize_cbor(&pledge_delta).unwrap(),
            TokenAmount::zero(),
            None,
            ExitCode::OK,
        );
    }

    let result = rt
        .call::<Actor>(
            Method::UpgradeSectorQuality as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap();
    let batch_return: BatchReturn = result.unwrap().deserialize().unwrap();
    rt.verify();

    assert_eq!(1, batch_return.success_count as usize);

    // After upgrade, daily fee should be non-zero (since circulating supply is non-zero).
    let sector_after = h.get_sector(&rt, sno);
    assert!(
        sector_after.daily_fee.is_positive(),
        "daily fee should be set for a pre-FIP-0100 legacy sector after upgrade"
    );
    assert!(sector_after.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));

    h.check_state(&rt);
}

/// Upgrading several legacy sectors together exercises the batched deadline/partition
/// lookup (State::find_sectors), not just the single-sector lookup path.
#[test]
fn upgrade_multiple_legacy_sectors_to_full_qa() {
    let (h, rt, sectors) = setup_sectors(3);
    let snos: Vec<u64> = sectors.iter().map(|s| s.sector_number).collect();

    for &sno in &snos {
        downgrade_sector_in_state(&h, &rt, sno, false);
    }

    let legacy_sectors: Vec<SectorOnChainInfo> =
        snos.iter().map(|&sno| h.get_sector(&rt, sno)).collect();
    for sector in &legacy_sectors {
        assert!(!sector.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    }

    let new_qa_power = qa_power_max(h.sector_size);
    let old_qa_power_total: BigInt = legacy_sectors
        .iter()
        .map(|s| qa_power_for_sector(h.sector_size, s))
        .fold(BigInt::zero(), |acc, p| acc + p);
    let qa_power_delta = BigInt::from(3) * &new_qa_power - old_qa_power_total;

    let new_pledge = h.initial_pledge_for_power(&rt, &new_qa_power);
    let pledge_delta: TokenAmount = legacy_sectors
        .iter()
        .map(|s| {
            if new_pledge > s.initial_pledge {
                &new_pledge - &s.initial_pledge
            } else {
                TokenAmount::zero()
            }
        })
        .fold(TokenAmount::zero(), |acc, d| acc + d);

    let bf = BitField::try_from_bits(snos.iter().copied()).unwrap();
    let params = UpgradeSectorQualityParams { sectors: bf };

    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
    rt.expect_validate_caller_addr(h.caller_addrs());
    h.expect_query_network_info(&rt);

    rt.expect_send_simple(
        STORAGE_POWER_ACTOR_ADDR,
        UPDATE_CLAIMED_POWER_METHOD,
        IpldBlock::serialize_cbor(&UpdateClaimedPowerParams {
            raw_byte_delta: BigInt::zero(),
            quality_adjusted_delta: qa_power_delta.clone(),
        })
        .unwrap(),
        TokenAmount::zero(),
        None,
        ExitCode::OK,
    );
    if pledge_delta.is_positive() {
        rt.expect_send_simple(
            STORAGE_POWER_ACTOR_ADDR,
            PowerMethod::UpdatePledgeTotal as u64,
            IpldBlock::serialize_cbor(&pledge_delta).unwrap(),
            TokenAmount::zero(),
            None,
            ExitCode::OK,
        );
    }

    let result = rt
        .call::<Actor>(
            Method::UpgradeSectorQuality as u64,
            IpldBlock::serialize_cbor(&params).unwrap(),
        )
        .unwrap();
    let batch_return: BatchReturn = result.unwrap().deserialize().unwrap();
    rt.verify();

    assert_eq!(3, batch_return.size());
    assert_eq!(3, batch_return.success_count as usize);
    assert!(batch_return.fail_codes.is_empty());

    for &sno in &snos {
        let sector_after = h.get_sector(&rt, sno);
        assert!(
            sector_after.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER),
            "sector {} should have FULL_QA_POWER flag after upgrade",
            sno
        );
        assert_eq!(qa_power_max(h.sector_size), qa_power_for_sector(h.sector_size, &sector_after));
    }

    h.check_state(&rt);
}
