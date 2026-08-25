use fil_actor_miner::{
    Actor, ExpirationExtension2, ExtendSectorExpiration2Params, Method, PoStPartition, PowerPair,
    SectorOnChainInfo, SectorOnChainInfoFlags, State, UpgradeSectorQuality,
    UpgradeSectorQualityParams, daily_proof_fee, daily_proof_fee_adjust,
    pledge_penalty_for_continued_fault, pledge_penalty_for_termination, power_for_sectors,
    qa_power_for_sector, qa_power_max,
};
use fil_actors_runtime::{
    EPOCHS_IN_DAY,
    reward::FilterEstimate,
    runtime::{Runtime, RuntimePolicy},
    test_utils::{ACCOUNT_ACTOR_CODE_ID, MockRuntime, expect_abort, expect_abort_contains_message},
};
use fvm_ipld_bitfield::BitField;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};

use num_traits::Zero;
use std::cmp::max;
use std::collections::BTreeMap;
use std::ops::Neg;

mod util;
use util::*;

// an expiration ~10 days greater than effective min expiration taking into account 30 days max between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

fn setup() -> (ActorHarness, MockRuntime) {
    let period_offset = 100;
    let precommit_epoch = 1;

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG512MiBV1);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    rt.set_epoch(precommit_epoch);

    (h, rt)
}

/// Rewrites proven sectors as pre-FIP-0118 sectors: the given flags (never `FULL_QA_POWER`),
/// `verified_space` bytes of verified data held for their whole life, and pledge and daily fee
/// at the rate of the resulting quality-adjusted power. Returns the rewritten records.
fn make_legacy(
    h: &ActorHarness,
    rt: &MockRuntime,
    sectors: &[SectorOnChainInfo],
    flags: SectorOnChainInfoFlags,
    verified_space: u64,
) -> Vec<SectorOnChainInfo> {
    let numbers: Vec<_> = sectors.iter().map(|s| s.sector_number).collect();
    h.rewrite_sectors(rt, &numbers, |sector| {
        sector.flags = flags;
        sector.verified_deal_weight =
            BigInt::from(verified_space) * (sector.expiration - sector.power_base_epoch);
        let qa_power = qa_power_for_sector(h.sector_size, sector);
        sector.initial_pledge = h.initial_pledge_for_power(rt, &qa_power);
        sector.daily_fee = daily_proof_fee(rt.policy(), &rt.total_fil_circ_supply(), &qa_power);
    });
    h.check_state(rt);
    numbers.iter().map(|&n| h.get_sector(rt, n)).collect()
}

/// Commits and proves `count` sectors, leaving their as-committed (full-power) records for
/// tests that fabricate their own legacy shape.
fn commit_proven_sectors(
    h: &mut ActorHarness,
    rt: &MockRuntime,
    count: usize,
) -> Vec<SectorOnChainInfo> {
    h.construct_and_verify(rt);
    let sectors =
        h.commit_and_prove_sectors(rt, count, DEFAULT_SECTOR_EXPIRATION as u64, Vec::new(), true);
    h.advance_and_submit_posts(rt, &sectors);
    sectors
}

/// Commits and proves `count` sectors, then rewrites them as legacy sectors of the given shape.
fn commit_legacy_sectors(
    h: &mut ActorHarness,
    rt: &MockRuntime,
    count: usize,
    flags: SectorOnChainInfoFlags,
    verified_space: u64,
) -> Vec<SectorOnChainInfo> {
    let sectors = commit_proven_sectors(h, rt, count);
    make_legacy(h, rt, &sectors, flags, verified_space)
}

/// A CC sector at 1x, as committed before FIP-0118.
fn commit_legacy_cc_sector(h: &mut ActorHarness, rt: &MockRuntime) -> SectorOnChainInfo {
    commit_legacy_sectors(h, rt, 1, SectorOnChainInfoFlags::SIMPLE_QA_POWER, 0).remove(0)
}

fn sector_location(rt: &MockRuntime, sector_number: SectorNumber) -> (u64, u64) {
    let state: State = rt.get_state();
    state.find_sector(rt.store(), sector_number).unwrap()
}

fn group_by_partition(
    rt: &MockRuntime,
    sectors: &[SectorOnChainInfo],
) -> BTreeMap<(u64, u64), Vec<SectorNumber>> {
    let mut by_partition: BTreeMap<(u64, u64), Vec<SectorNumber>> = BTreeMap::new();
    for sector in sectors {
        by_partition
            .entry(sector_location(rt, sector.sector_number))
            .or_default()
            .push(sector.sector_number);
    }
    by_partition
}

/// One upgrade-only declaration per (deadline, partition) home of the sectors.
fn upgrade_only_declarations(
    rt: &MockRuntime,
    sectors: &[SectorOnChainInfo],
) -> Vec<UpgradeSectorQuality> {
    group_by_partition(rt, sectors)
        .into_iter()
        .map(|((deadline, partition), sectors)| UpgradeSectorQuality {
            deadline,
            partition,
            sectors: make_bitfield(&sectors),
            new_expiration: None,
        })
        .collect()
}

fn upgrade_params(
    rt: &MockRuntime,
    sector_number: SectorNumber,
    new_expiration: Option<ChainEpoch>,
) -> UpgradeSectorQualityParams {
    let (deadline, partition) = sector_location(rt, sector_number);
    UpgradeSectorQualityParams {
        extensions: vec![UpgradeSectorQuality {
            deadline,
            partition,
            sectors: make_bitfield(&[sector_number]),
            new_expiration,
        }],
    }
}

/// Power and pledge deltas expected from upgrading these sectors to full power: power rises to
/// the maximum, pledge to `max(old, 10x requirement)`.
fn expected_upgrade_deltas(
    h: &ActorHarness,
    rt: &MockRuntime,
    sectors: &[SectorOnChainInfo],
) -> (PowerPair, TokenAmount) {
    let pledge_10x = h.initial_pledge_for_power(rt, &qa_power_max(h.sector_size));
    let mut qa_delta = BigInt::zero();
    let mut pledge_delta = TokenAmount::zero();
    for sector in sectors {
        qa_delta += qa_power_max(h.sector_size) - qa_power_for_sector(h.sector_size, sector);
        pledge_delta += max(&pledge_10x, &sector.initial_pledge) - &sector.initial_pledge;
    }
    (PowerPair::new(BigInt::zero(), qa_delta), pledge_delta)
}

/// The daily fee a sector carries after its upgrade: attached at the full rate if it had none
/// (pre-FIP-0100), otherwise scaled from its old power to full power.
fn upgraded_fee(h: &ActorHarness, rt: &MockRuntime, sector: &SectorOnChainInfo) -> TokenAmount {
    let full_power = qa_power_max(h.sector_size);
    if sector.daily_fee.is_zero() {
        daily_proof_fee(rt.policy(), &rt.total_fil_circ_supply(), &full_power)
    } else {
        let old_power = qa_power_for_sector(h.sector_size, sector);
        daily_proof_fee_adjust(&sector.daily_fee, &old_power, &full_power)
    }
}

/// Asserts `upgraded` carries full power's flags, pledge and daily fee, given the legacy record
/// it replaced.
fn assert_full_power_record(
    h: &ActorHarness,
    rt: &MockRuntime,
    legacy: &SectorOnChainInfo,
    upgraded: &SectorOnChainInfo,
) {
    assert!(
        upgraded.flags.contains(
            SectorOnChainInfoFlags::SIMPLE_QA_POWER | SectorOnChainInfoFlags::FULL_QA_POWER
        )
    );
    assert_eq!(qa_power_max(h.sector_size), qa_power_for_sector(h.sector_size, upgraded));
    let pledge_10x = h.initial_pledge_for_power(rt, &qa_power_max(h.sector_size));
    assert_eq!(max(&pledge_10x, &legacy.initial_pledge), &upgraded.initial_pledge);
    assert_eq!(upgraded_fee(h, rt, legacy), upgraded.daily_fee);
}

#[test]
fn upgrade_only_upgrades_in_place() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let (deadline_index, _) = sector_location(&rt, legacy.sector_number);

    let state_before: State = rt.get_state();
    let deadline_before = h.get_deadline(&rt, deadline_index);

    // A 1x sector gains nine more units of QA power and the pledge top-up to match.
    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    assert_eq!(BigInt::from(h.sector_size as u64 * 9), power_delta.qa);
    assert!(pledge_delta.is_positive());

    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta.clone(),
    )
    .unwrap();

    // Only the flags, pledge and fee changed on the record.
    let upgraded = h.get_sector(&rt, legacy.sector_number);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(legacy.expiration, upgraded.expiration);
    assert_eq!(legacy.power_base_epoch, upgraded.power_base_epoch);
    assert_eq!(legacy.deal_weight, upgraded.deal_weight);
    assert_eq!(legacy.verified_deal_weight, upgraded.verified_deal_weight);

    // The miner's pledge total grew by the top-up and equals the record's pledge, so the
    // sector's eventual expiry releases exactly what is locked.
    let state_after: State = rt.get_state();
    assert_eq!(pledge_delta, &state_after.initial_pledge - &state_before.initial_pledge);
    assert_eq!(upgraded.initial_pledge, state_after.initial_pledge);

    // No expiration moved, so the deadline's schedule was not rewritten.
    let deadline_after = h.get_deadline(&rt, deadline_index);
    assert_eq!(deadline_before.expirations_epochs, deadline_after.expirations_epochs);
    h.check_state(&rt);
}

#[test]
fn upgrade_with_extension_in_one_declaration() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let (deadline_index, partition_index) = sector_location(&rt, legacy.sector_number);
    let new_expiration = legacy.expiration + 42 * EPOCHS_IN_DAY;

    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, Some(new_expiration)),
        power_delta,
        pledge_delta,
    )
    .unwrap();

    let upgraded = h.get_sector(&rt, legacy.sector_number);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(new_expiration, upgraded.expiration);
    assert_eq!(*rt.epoch.borrow(), upgraded.power_base_epoch);

    // The partition's expiration queue holds the sector at the new expiration and nowhere
    // earlier.
    let state: State = rt.get_state();
    let quant = state.quant_spec_for_deadline(rt.policy(), deadline_index);
    let (_, mut partition) = h.get_deadline_and_partition(&rt, deadline_index, partition_index);
    assert!(
        partition.pop_expired_sectors(rt.store(), new_expiration - 1, quant).unwrap().is_empty()
    );
    let expiring = partition
        .pop_expired_sectors(rt.store(), quant.quantize_up(new_expiration), quant)
        .unwrap();
    assert!(expiring.on_time_sectors.get(upgraded.sector_number));
    h.check_state(&rt);
}

#[test]
fn repeated_upgrade_is_a_no_op() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);

    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();
    let upgraded = h.get_sector(&rt, legacy.sector_number);
    let state_root = *rt.state.borrow();

    // A sector at full power is skipped: neither an in-place repeat nor one asking for an
    // extension moves power, pledge, fee or expiration, and state stays byte-identical.
    for new_expiration in [None, Some(upgraded.expiration + 42 * EPOCHS_IN_DAY)] {
        h.upgrade_sector_quality(
            &rt,
            upgrade_params(&rt, upgraded.sector_number, new_expiration),
            PowerPair::zero(),
            TokenAmount::zero(),
        )
        .unwrap();
        assert_eq!(state_root, *rt.state.borrow());
    }
    h.check_state(&rt);
}

#[test]
fn repeated_upgrade_locks_nothing_when_pledge_requirement_rises() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);

    // Depress the pledge requirement below the sector's recorded pledge for the first
    // upgrade: zero supply removes the supply share, and a small reward keeps the base term
    // under the per-byte pledge cap.
    let normal_supply = rt.total_fil_circ_supply();
    let normal_reward = h.epoch_reward_smooth.clone();
    rt.set_circulating_supply(TokenAmount::zero());
    h.epoch_reward_smooth =
        FilterEstimate::new(TokenAmount::from_nano(10_000_000).atto().clone(), BigInt::zero());

    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    assert!(pledge_delta.is_zero());
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();
    let after_first = h.get_sector(&rt, legacy.sector_number);

    // Conditions recover, so the full-power requirement now exceeds what the sector holds.
    // Already at full power, a repeat must lock nothing.
    rt.set_circulating_supply(normal_supply);
    h.epoch_reward_smooth = normal_reward;
    let raised_requirement = h.initial_pledge_for_power(&rt, &qa_power_max(h.sector_size));
    assert!(raised_requirement > after_first.initial_pledge);

    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        PowerPair::zero(),
        TokenAmount::zero(),
    )
    .unwrap();
    assert_eq!(after_first, h.get_sector(&rt, legacy.sector_number));
    h.check_state(&rt);
}

#[test]
fn upgrades_partially_verified_pre_fip0045_sector() {
    let (mut h, rt) = setup();
    // Half the sector holds verified data, and it predates FIP-0045 so it carries no flag.
    let half = h.sector_size as u64 / 2;
    let legacy =
        commit_legacy_sectors(&mut h, &rt, 1, SectorOnChainInfoFlags::empty(), half).remove(0);
    assert_eq!(
        BigInt::from(h.sector_size as u64 * 11 / 2),
        qa_power_for_sector(h.sector_size, &legacy),
        "half verified is 5.5x"
    );

    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();

    // Full power, with pledge and fee scaled up from 5.5x; the weights stay as they were, now
    // masked by the flag.
    let upgraded = h.get_sector(&rt, legacy.sector_number);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(legacy.verified_deal_weight, upgraded.verified_deal_weight);
    assert_eq!(legacy.power_base_epoch, upgraded.power_base_epoch);
    h.check_state(&rt);
}

#[test]
fn upgrades_sector_with_unverified_data_keeping_weights() {
    let (mut h, rt) = setup();
    let sector = commit_proven_sectors(&mut h, &rt, 1).remove(0);

    // A legacy sector half-full of unverified deal data. Deal weight has no power effect, so
    // it counts 1x before the upgrade, like a CC sector.
    let raw_power = StoragePower::from(h.sector_size as u64);
    let pledge_1x = h.initial_pledge_for_power(&rt, &raw_power);
    let fee_1x = daily_proof_fee(rt.policy(), &rt.total_fil_circ_supply(), &raw_power);
    let half_space = BigInt::from(h.sector_size as u64 / 2);
    h.rewrite_sectors(&rt, &[sector.sector_number], |s| {
        s.flags = SectorOnChainInfoFlags::SIMPLE_QA_POWER;
        s.deal_weight = &half_space * (s.expiration - s.power_base_epoch);
        s.initial_pledge = pledge_1x.clone();
        s.daily_fee = fee_1x.clone();
    });
    let legacy = h.get_sector(&rt, sector.sector_number);
    assert_eq!(raw_power, qa_power_for_sector(h.sector_size, &legacy));
    h.check_state(&rt);

    // The upgrade is the same full 9x jump as for a CC sector, and the deal weight survives
    // untouched.
    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    assert_eq!(BigInt::from(h.sector_size as u64 * 9), power_delta.qa);
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();

    let upgraded = h.get_sector(&rt, legacy.sector_number);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(legacy.deal_weight, upgraded.deal_weight);
    assert!(upgraded.verified_deal_weight.is_zero());
    h.check_state(&rt);
}

#[test]
fn extension_restates_verified_weight_and_grants_full_power() {
    let (mut h, rt) = setup();
    let half = h.sector_size as u64 / 2;
    let legacy =
        commit_legacy_sectors(&mut h, &rt, 1, SectorOnChainInfoFlags::SIMPLE_QA_POWER, half)
            .remove(0);
    let curr_epoch = *rt.epoch.borrow();
    let new_expiration = legacy.expiration + 42 * EPOCHS_IN_DAY;

    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, Some(new_expiration)),
        power_delta,
        pledge_delta,
    )
    .unwrap();

    // Same weight math as ExtendSectorExpiration2: the verified space is restated over the new
    // duration. Power comes from the flag, so the restated weight is not what raised it.
    let upgraded = h.get_sector(&rt, legacy.sector_number);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(new_expiration, upgraded.expiration);
    assert_eq!(curr_epoch, upgraded.power_base_epoch);
    let old_duration = legacy.expiration - legacy.power_base_epoch;
    let new_duration = new_expiration - curr_epoch;
    assert_eq!(
        (&legacy.verified_deal_weight / old_duration) * new_duration,
        upgraded.verified_deal_weight
    );
    h.check_state(&rt);
}

#[test]
fn already_full_power_by_weights_is_skipped() {
    let (mut h, rt) = setup();
    // Already 10x through its weights alone, so there is nothing to raise: the sector is not
    // eligible and is skipped — not even flagged — leaving state byte-identical.
    let full = h.sector_size as u64;
    let legacy =
        commit_legacy_sectors(&mut h, &rt, 1, SectorOnChainInfoFlags::SIMPLE_QA_POWER, full)
            .remove(0);
    assert_eq!(qa_power_max(h.sector_size), qa_power_for_sector(h.sector_size, &legacy));
    let state_root = *rt.state.borrow();

    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        PowerPair::zero(),
        TokenAmount::zero(),
    )
    .unwrap();

    assert_eq!(legacy, h.get_sector(&rt, legacy.sector_number));
    assert_eq!(state_root, *rt.state.borrow());
    h.check_state(&rt);
}

#[test]
fn keeps_higher_existing_pledge_without_topping_up() {
    let (mut h, rt) = setup();
    let sector = commit_proven_sectors(&mut h, &rt, 1).remove(0);

    // The sector already holds more pledge than today's 10x requirement (e.g. it onboarded
    // when pledge ran higher). The rule is max(old, new): nothing extra to lock, and the
    // pledge is never lowered.
    let raw_power = StoragePower::from(h.sector_size as u64);
    let pledge_10x = h.initial_pledge_for_power(&rt, &qa_power_max(h.sector_size));
    let high_pledge = &pledge_10x * 2;
    let fee_1x = daily_proof_fee(rt.policy(), &rt.total_fil_circ_supply(), &raw_power);
    h.rewrite_sectors(&rt, &[sector.sector_number], |s| {
        s.flags = SectorOnChainInfoFlags::SIMPLE_QA_POWER;
        s.initial_pledge = high_pledge.clone();
        s.daily_fee = fee_1x.clone();
    });
    let legacy = h.get_sector(&rt, sector.sector_number);
    h.check_state(&rt);

    let state_before: State = rt.get_state();
    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    assert!(pledge_delta.is_zero());
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();

    // Power and fee still move to the 10x level; the pledge stays put.
    let upgraded = h.get_sector(&rt, legacy.sector_number);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(high_pledge, upgraded.initial_pledge);
    let state_after: State = rt.get_state();
    assert_eq!(state_before.initial_pledge, state_after.initial_pledge);
    h.check_state(&rt);
}

#[test]
fn clears_deprecated_reward_estimates_on_upgrade() {
    let (mut h, rt) = setup();
    let sectors = commit_legacy_sectors(&mut h, &rt, 2, SectorOnChainInfoFlags::SIMPLE_QA_POWER, 0);
    let numbers = [sectors[0].sector_number, sectors[1].sector_number];
    let (deadline, partition) = sector_location(&rt, numbers[0]);
    assert_eq!((deadline, partition), sector_location(&rt, numbers[1]));

    // Real pre-FIP-0100 records still carry the deprecated estimates.
    h.rewrite_sectors(&rt, &numbers, |s| {
        s.expected_day_reward = Some(TokenAmount::from_whole(1));
        s.expected_storage_pledge = Some(TokenAmount::from_whole(2));
        s.replaced_day_reward = Some(TokenAmount::from_whole(3));
    });
    let legacy: Vec<_> = numbers.iter().map(|&n| h.get_sector(&rt, n)).collect();

    // One sector upgrades in place, the other also extends: both paths clear the estimates.
    let extensions = vec![
        UpgradeSectorQuality {
            deadline,
            partition,
            sectors: make_bitfield(&[numbers[0]]),
            new_expiration: None,
        },
        UpgradeSectorQuality {
            deadline,
            partition,
            sectors: make_bitfield(&[numbers[1]]),
            new_expiration: Some(legacy[1].expiration + 42 * EPOCHS_IN_DAY),
        },
    ];
    let (power_delta, pledge_delta) = expected_upgrade_deltas(&h, &rt, &legacy);
    h.upgrade_sector_quality(
        &rt,
        UpgradeSectorQualityParams { extensions },
        power_delta,
        pledge_delta,
    )
    .unwrap();

    for &number in &numbers {
        let upgraded = h.get_sector(&rt, number);
        assert_eq!(None, upgraded.expected_day_reward);
        assert_eq!(None, upgraded.expected_storage_pledge);
        assert_eq!(None, upgraded.replaced_day_reward);
    }
    h.check_state(&rt);
}

#[test]
fn one_declaration_upgrades_sectors_with_different_expirations() {
    let (mut h, rt) = setup();
    let sectors = commit_proven_sectors(&mut h, &rt, 2);
    let (deadline, partition) = sector_location(&rt, sectors[0].sector_number);
    assert_eq!((deadline, partition), sector_location(&rt, sectors[1].sector_number));

    // Push one sector's expiration out so the partition holds two different ones.
    h.extend_sectors2(
        &rt,
        ExtendSectorExpiration2Params {
            extensions: vec![ExpirationExtension2 {
                deadline,
                partition,
                sectors: make_bitfield(&[sectors[1].sector_number]),
                sectors_with_claims: vec![],
                new_expiration: sectors[1].expiration + 40 * EPOCHS_IN_DAY,
            }],
        },
    )
    .unwrap();
    let sectors: Vec<_> = sectors.iter().map(|s| h.get_sector(&rt, s.sector_number)).collect();
    let legacy = make_legacy(&h, &rt, &sectors, SectorOnChainInfoFlags::SIMPLE_QA_POWER, 0);
    assert_ne!(legacy[0].expiration, legacy[1].expiration);

    // One upgrade-only declaration covers both expirations at once.
    let params = UpgradeSectorQualityParams {
        extensions: vec![UpgradeSectorQuality {
            deadline,
            partition,
            sectors: make_bitfield(&[legacy[0].sector_number, legacy[1].sector_number]),
            new_expiration: None,
        }],
    };
    let (power_delta, pledge_delta) = expected_upgrade_deltas(&h, &rt, &legacy);
    h.upgrade_sector_quality(&rt, params, power_delta, pledge_delta).unwrap();

    for sector in &legacy {
        let upgraded = h.get_sector(&rt, sector.sector_number);
        assert_full_power_record(&h, &rt, sector, &upgraded);
        assert_eq!(sector.expiration, upgraded.expiration);
    }
    h.check_state(&rt);
}

#[test]
fn extension_skips_sectors_already_at_full_power() {
    let (mut h, rt) = setup();
    let sectors = commit_proven_sectors(&mut h, &rt, 2);
    let (deadline, partition) = sector_location(&rt, sectors[0].sector_number);
    assert_eq!((deadline, partition), sector_location(&rt, sectors[1].sector_number));

    // One sector is made legacy; its partition-mate keeps its as-committed full power.
    let legacy =
        make_legacy(&h, &rt, &sectors[..1], SectorOnChainInfoFlags::SIMPLE_QA_POWER, 0).remove(0);
    let full = h.get_sector(&rt, sectors[1].sector_number);
    assert!(full.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));

    // One declaration extends both. Only the legacy sector is upgraded and extended; the
    // full-power one is not eligible, so it keeps its expiration (ExtendSectorExpiration2
    // is the way to extend it).
    let new_expiration = legacy.expiration + 42 * EPOCHS_IN_DAY;
    let params = UpgradeSectorQualityParams {
        extensions: vec![UpgradeSectorQuality {
            deadline,
            partition,
            sectors: make_bitfield(&[legacy.sector_number, full.sector_number]),
            new_expiration: Some(new_expiration),
        }],
    };
    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(&rt, params, power_delta, pledge_delta).unwrap();

    let upgraded = h.get_sector(&rt, legacy.sector_number);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(new_expiration, upgraded.expiration);
    assert_eq!(full, h.get_sector(&rt, full.sector_number));
    h.check_state(&rt);
}

#[test]
fn mixed_batch_upgrades_extends_and_requeues_across_epochs() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_sectors(&mut h, &rt, 4, SectorOnChainInfoFlags::SIMPLE_QA_POWER, 0);

    // The batch needs one partition holding at least two sectors and one other partition.
    let by_partition = group_by_partition(&rt, &legacy);
    let (&(deadline, partition), pair) = by_partition
        .iter()
        .find(|(_, sectors)| sectors.len() >= 2)
        .expect("need a partition with two sectors");
    let (&(other_deadline, other_partition), other) = by_partition
        .iter()
        .find(|&(&key, _)| key != (deadline, partition))
        .expect("need a second partition");
    let by_number: BTreeMap<u64, &SectorOnChainInfo> =
        legacy.iter().map(|s| (s.sector_number, s)).collect();

    // Two different epochs inside one partition, the first epoch declared twice (the repeat
    // finds its sector already at full power and skips it), an in-place upgrade in the other
    // partition, and that partition's second sector left out of the batch entirely.
    let base = max(by_number[&pair[0]].expiration, by_number[&pair[1]].expiration);
    let epoch_one = base + 30 * EPOCHS_IN_DAY;
    let epoch_two = base + 60 * EPOCHS_IN_DAY;
    let extensions = vec![
        UpgradeSectorQuality {
            deadline,
            partition,
            sectors: make_bitfield(&[pair[0]]),
            new_expiration: Some(epoch_one),
        },
        UpgradeSectorQuality {
            deadline,
            partition,
            sectors: make_bitfield(&[pair[1]]),
            new_expiration: Some(epoch_two),
        },
        UpgradeSectorQuality {
            deadline,
            partition,
            sectors: make_bitfield(&[pair[0]]),
            new_expiration: Some(epoch_one),
        },
        UpgradeSectorQuality {
            deadline: other_deadline,
            partition: other_partition,
            sectors: make_bitfield(&[other[0]]),
            new_expiration: None,
        },
    ];

    let (power_delta, pledge_delta) = expected_upgrade_deltas(
        &h,
        &rt,
        &[by_number[&pair[0]].clone(), by_number[&pair[1]].clone(), by_number[&other[0]].clone()],
    );
    h.upgrade_sector_quality(
        &rt,
        UpgradeSectorQualityParams { extensions },
        power_delta,
        pledge_delta,
    )
    .unwrap();

    // Every declared sector is upgraded; the extended ones landed on their own epochs, the
    // in-place one kept its schedule, and the undeclared one is untouched. check_state then
    // rebuilds each partition's expiration sets from the records (power, pledge, fee, epoch)
    // and requires every set's epoch to be registered in the deadline's queue: the requeue
    // bookkeeping for both new epochs.
    let first = h.get_sector(&rt, pair[0]);
    let second = h.get_sector(&rt, pair[1]);
    let in_place = h.get_sector(&rt, other[0]);
    assert_full_power_record(&h, &rt, by_number[&pair[0]], &first);
    assert_full_power_record(&h, &rt, by_number[&pair[1]], &second);
    assert_full_power_record(&h, &rt, by_number[&other[0]], &in_place);
    assert_eq!(epoch_one, first.expiration);
    assert_eq!(epoch_two, second.expiration);
    assert_eq!(by_number[&other[0]].expiration, in_place.expiration);
    for &bystander in &other[1..] {
        assert_eq!(*by_number[&bystander], h.get_sector(&rt, bystander));
    }
    h.check_state(&rt);
}

#[test]
fn duplicate_sector_across_declarations_upgrades_once() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let (deadline, partition) = sector_location(&rt, legacy.sector_number);

    let decl = UpgradeSectorQuality {
        deadline,
        partition,
        sectors: make_bitfield(&[legacy.sector_number]),
        new_expiration: None,
    };
    let params = UpgradeSectorQualityParams { extensions: vec![decl.clone(), decl] };

    // Exactly one power bump and one pledge raise despite two declarations.
    let state_before: State = rt.get_state();
    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(&rt, params, power_delta, pledge_delta.clone()).unwrap();

    let state_after: State = rt.get_state();
    assert_eq!(pledge_delta, &state_after.initial_pledge - &state_before.initial_pledge);
    assert_full_power_record(&h, &rt, &legacy, &h.get_sector(&rt, legacy.sector_number));
    h.check_state(&rt);
}

#[test]
fn deadline_cron_charges_the_upgraded_fee() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);

    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();
    let upgraded = h.get_sector(&rt, legacy.sector_number);
    assert!(upgraded.daily_fee > legacy.daily_fee);

    // The deadline's fee book now carries the upgraded fee. Driving the next proving period
    // (PoSt at the sector's deadline, cron at every deadline) makes the actor charge that book
    // at the sector's deadline cron; the harness expects the burn to f099, and any vesting-funds
    // draw that pays it, computed from the record, to the atto.
    let (deadline_index, _) = sector_location(&rt, legacy.sector_number);
    assert_eq!(upgraded.daily_fee, h.get_deadline(&rt, deadline_index).daily_fee);
    h.advance_and_submit_posts(&rt, std::slice::from_ref(&upgraded));
    h.check_state(&rt);
}

#[test]
fn deadline_cron_releases_upgraded_power_and_pledge_at_expiration() {
    // Bespoke setup with a one-period minimum lifetime, so the sector's whole life fits in a
    // few proving periods of cron driving.
    let mut h = ActorHarness::new(100);
    h.set_proof_type(RegisteredSealProof::StackedDRG512MiBV1);
    let mut rt = h.new_runtime();
    rt.policy.min_sector_expiration = rt.policy.wpost_proving_period;
    rt.balance.replace(BIG_BALANCE.clone());
    rt.set_epoch(1);

    h.construct_and_verify(&rt);
    let sector = h.commit_and_prove_sectors(&rt, 1, 3, Vec::new(), true).remove(0);
    h.advance_and_submit_posts(&rt, std::slice::from_ref(&sector));
    let legacy = make_legacy(
        &h,
        &rt,
        std::slice::from_ref(&sector),
        SectorOnChainInfoFlags::SIMPLE_QA_POWER,
        0,
    )
    .remove(0);

    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();
    let upgraded = h.get_sector(&rt, legacy.sector_number);
    let (deadline_index, partition_index) = sector_location(&rt, upgraded.sector_number);

    // Keep the sector proven until the period holding its expiration.
    while *rt.epoch.borrow() + rt.policy.wpost_proving_period < upgraded.expiration {
        h.advance_and_submit_posts(&rt, std::slice::from_ref(&upgraded));
    }

    // Prove the final window, then let its deadline cron pop the expiration: it must release
    // exactly the upgraded power and the topped-up pledge.
    let final_window = h.advance_to_deadline(&rt, deadline_index);
    h.submit_window_post(
        &rt,
        &final_window,
        vec![PoStPartition { index: partition_index, skipped: BitField::new() }],
        vec![upgraded.clone()],
        PoStConfig::with_expected_power_delta(&PowerPair::zero()),
    );
    let power_10x = power_for_sectors(h.sector_size, std::slice::from_ref(&upgraded));
    assert_eq!(qa_power_max(h.sector_size), power_10x.qa);
    h.advance_deadline(
        &rt,
        CronConfig {
            no_enrollment: true,
            power_delta: Some(power_10x.neg()),
            pledge_delta: upgraded.initial_pledge.clone().neg(),
            ..CronConfig::empty()
        },
    );

    // The sector is out of the proving set (its record lingers until cleanup) and the miner
    // holds no pledge any more.
    let (_, partition) = h.get_deadline_and_partition(&rt, deadline_index, partition_index);
    assert!(partition.terminated.get(upgraded.sector_number));
    assert!(partition.live_power.is_zero());
    let state: State = rt.get_state();
    assert!(state.initial_pledge.is_zero());
    h.check_state(&rt);
}

#[test]
fn terminates_upgraded_sector_at_upgraded_pledge_and_power() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);

    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();
    let upgraded = h.get_sector(&rt, legacy.sector_number);

    // Locked rewards ensure the fee is unlockable, as in the terminate tests.
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());

    // The termination fee's fault-fee floor runs on the upgraded (10x) power, and the released
    // pledge is the topped-up one.
    let sector_power = qa_power_for_sector(h.sector_size, &upgraded);
    assert_eq!(qa_power_max(h.sector_size), sector_power);
    let fault_fee = pledge_penalty_for_continued_fault(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &sector_power,
    );
    let expected_fee = pledge_penalty_for_termination(
        &upgraded.initial_pledge,
        *rt.epoch.borrow() - upgraded.activation,
        &fault_fee,
    );
    let (power_removed, pledge_removed) =
        h.terminate_sectors(&rt, &make_bitfield(&[upgraded.sector_number]), expected_fee.clone());
    assert_eq!(
        power_for_sectors(h.sector_size, std::slice::from_ref(&upgraded)).neg(),
        power_removed
    );
    assert_eq!(-(expected_fee + &upgraded.initial_pledge), pledge_removed);

    let state: State = rt.get_state();
    assert!(state.initial_pledge.is_zero());
    h.check_state(&rt);
}

#[test]
fn extend2_after_upgrade_keeps_full_power_and_pledge() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);

    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();
    let upgraded = h.get_sector(&rt, legacy.sector_number);

    // A later plain extension keeps the 10x contract: flag, pledge and fee ride along; only
    // the expiration and power base move.
    let (deadline, partition) = sector_location(&rt, upgraded.sector_number);
    let new_expiration = upgraded.expiration + 42 * EPOCHS_IN_DAY;
    h.extend_sectors2(
        &rt,
        ExtendSectorExpiration2Params {
            extensions: vec![ExpirationExtension2 {
                deadline,
                partition,
                sectors: make_bitfield(&[upgraded.sector_number]),
                sectors_with_claims: vec![],
                new_expiration,
            }],
        },
    )
    .unwrap();

    let extended = h.get_sector(&rt, upgraded.sector_number);
    assert_eq!(new_expiration, extended.expiration);
    assert_eq!(*rt.epoch.borrow(), extended.power_base_epoch);
    assert_eq!(upgraded.flags, extended.flags);
    assert_eq!(upgraded.initial_pledge, extended.initial_pledge);
    assert_eq!(upgraded.daily_fee, extended.daily_fee);
    assert_eq!(qa_power_max(h.sector_size), qa_power_for_sector(h.sector_size, &extended));
    h.check_state(&rt);
}

#[test]
fn withdraw_after_upgrade_leaves_the_new_pledge_locked() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);

    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();

    // Everything above the pledge and vesting funds can leave; the top-up cannot.
    let state: State = rt.get_state();
    let free =
        rt.get_balance() - &state.initial_pledge - &state.locked_funds - &state.pre_commit_deposits;
    h.withdraw_funds(&rt, h.owner, &rt.get_balance(), &free, &TokenAmount::zero()).unwrap();
    let pledge_10x = h.initial_pledge_for_power(&rt, &qa_power_max(h.sector_size));
    assert_eq!(pledge_10x, rt.get_state::<State>().initial_pledge);
    h.check_state(&rt);
}

#[test]
fn attaches_full_rate_fee_to_pre_fip0100_sector() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    h.rewrite_sectors(&rt, &[legacy.sector_number], |sector| {
        sector.daily_fee = TokenAmount::zero()
    });
    let legacy = h.get_sector(&rt, legacy.sector_number);

    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();

    let expected_fee =
        daily_proof_fee(rt.policy(), &rt.total_fil_circ_supply(), &qa_power_max(h.sector_size));
    assert!(expected_fee.is_positive());
    let upgraded = h.get_sector(&rt, legacy.sector_number);
    assert_eq!(expected_fee, upgraded.daily_fee);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    h.check_state(&rt);
}

#[test]
fn extend_sector_expiration2_does_not_upgrade() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let (deadline, partition) = sector_location(&rt, legacy.sector_number);

    let new_expiration = legacy.expiration + 42 * EPOCHS_IN_DAY;
    let params = ExtendSectorExpiration2Params {
        extensions: vec![ExpirationExtension2 {
            deadline,
            partition,
            sectors: make_bitfield(&[legacy.sector_number]),
            sectors_with_claims: vec![],
            new_expiration,
        }],
    };
    h.extend_sectors2(&rt, params).unwrap();

    // The plain extension contract holds: no flag, no pledge or fee change.
    let extended = h.get_sector(&rt, legacy.sector_number);
    assert!(!extended.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    assert_eq!(new_expiration, extended.expiration);
    assert_eq!(legacy.initial_pledge, extended.initial_pledge);
    assert_eq!(legacy.daily_fee, extended.daily_fee);
    h.check_state(&rt);
}

#[test]
fn rejects_empty_declaration_list() {
    let (h, rt) = setup();
    h.construct_and_verify(&rt);

    let res = h.upgrade_sector_quality(
        &rt,
        UpgradeSectorQualityParams { extensions: vec![] },
        PowerPair::zero(),
        TokenAmount::zero(),
    );
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "no extension declarations", res);
    rt.reset();
    h.check_state(&rt);
}

#[test]
fn rejects_empty_sector_selection() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let (deadline_index, partition) = sector_location(&rt, legacy.sector_number);
    let deadline_before = h.get_deadline(&rt, deadline_index);

    // Even a huge requested expiration must fail cleanly without touching state.
    for new_expiration in [None, Some(i64::MAX)] {
        let params = UpgradeSectorQualityParams {
            extensions: vec![UpgradeSectorQuality {
                deadline: deadline_index,
                partition,
                sectors: BitField::new(),
                new_expiration,
            }],
        };
        let res = h.upgrade_sector_quality(&rt, params, PowerPair::zero(), TokenAmount::zero());
        expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "no sectors selected", res);
        rt.reset();
    }

    let deadline_after = h.get_deadline(&rt, deadline_index);
    assert_eq!(deadline_before.expirations_epochs, deadline_after.expirations_epochs);
    h.check_state(&rt);
}

#[test]
fn rejects_expiration_at_or_before_current_epoch() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let curr_epoch = *rt.epoch.borrow();

    for bad_expiration in [0, -1, curr_epoch] {
        let res = h.upgrade_sector_quality(
            &rt,
            upgrade_params(&rt, legacy.sector_number, Some(bad_expiration)),
            PowerPair::zero(),
            TokenAmount::zero(),
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "must be after current epoch",
            res,
        );
        rt.reset();
    }

    let after = h.get_sector(&rt, legacy.sector_number);
    assert!(!after.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    h.check_state(&rt);
}

#[test]
fn rejects_expiration_beyond_max_extension() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let too_far = *rt.epoch.borrow() + rt.policy().max_sector_expiration_extension + 1;

    let res = h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, Some(too_far)),
        PowerPair::zero(),
        TokenAmount::zero(),
    );
    expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "cannot be more than", res);
    rt.reset();
    h.check_state(&rt);
}

#[test]
fn rejects_reducing_expiration() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);

    let res = h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, Some(legacy.expiration - 1)),
        PowerPair::zero(),
        TokenAmount::zero(),
    );
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!("cannot reduce sector {} expiration", legacy.sector_number),
        res,
    );
    rt.reset();
    h.check_state(&rt);
}

#[test]
fn rejects_unknown_deadline_partition_and_sector() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let (deadline, partition) = sector_location(&rt, legacy.sector_number);

    let cases = [
        (
            rt.policy().wpost_period_deadlines,
            partition,
            legacy.sector_number,
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "not in range",
        ),
        (
            deadline,
            partition + 1,
            legacy.sector_number,
            ExitCode::USR_NOT_FOUND,
            "no such partition",
        ),
        (
            deadline,
            partition,
            legacy.sector_number + 1,
            ExitCode::USR_NOT_FOUND,
            "sector not found",
        ),
    ];
    for (deadline, partition, sector_number, code, message) in cases {
        let params = UpgradeSectorQualityParams {
            extensions: vec![UpgradeSectorQuality {
                deadline,
                partition,
                sectors: make_bitfield(&[sector_number]),
                new_expiration: None,
            }],
        };
        let res = h.upgrade_sector_quality(&rt, params, PowerPair::zero(), TokenAmount::zero());
        expect_abort_contains_message(code, message, res);
        rt.reset();
    }

    let after = h.get_sector(&rt, legacy.sector_number);
    assert!(!after.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    h.check_state(&rt);
}

#[test]
fn faulty_declaration_aborts_the_whole_batch() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_sectors(&mut h, &rt, 3, SectorOnChainInfoFlags::SIMPLE_QA_POWER, 0);
    let faulty = legacy[2].clone();
    h.declare_faults(&rt, std::slice::from_ref(&faulty));

    // Good declarations first, the faulty sector's declaration last: the good partitions are
    // processed before the abort, and must still roll back.
    let mut declarations = upgrade_only_declarations(&rt, &legacy);
    declarations.sort_by_key(|d| d.sectors.get(faulty.sector_number));
    assert!(declarations.len() >= 2, "need the faulty sector in its own declaration");

    let res = h.upgrade_sector_quality(
        &rt,
        UpgradeSectorQualityParams { extensions: declarations },
        PowerPair::zero(),
        TokenAmount::zero(),
    );
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "can only upgrade active sectors",
        res,
    );
    rt.reset();

    // No sector was upgraded, including the ones in the good declarations.
    for sector in &legacy {
        let after = h.get_sector(&rt, sector.sector_number);
        assert!(!after.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
        assert_eq!(sector.initial_pledge, after.initial_pledge);
    }
    h.check_state(&rt);
}

#[test]
fn rejects_unproven_sector() {
    let (mut h, rt) = setup();
    h.construct_and_verify(&rt);
    // Committed but not yet proven in a WindowPoSt, so not yet active.
    let sector = h
        .commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, Vec::new(), true)
        .remove(0);

    let res = h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, sector.sector_number, None),
        PowerPair::zero(),
        TokenAmount::zero(),
    );
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "can only upgrade active sectors",
        res,
    );
    rt.reset();
    h.check_state(&rt);
}

#[test]
fn rejects_sector_from_another_partition() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_sectors(&mut h, &rt, 3, SectorOnChainInfoFlags::SIMPLE_QA_POWER, 0);
    let by_partition = group_by_partition(&rt, &legacy);
    assert!(by_partition.len() >= 2, "test needs sectors in more than one partition");

    // A sector that exists, declared under a partition that does not hold it.
    let (&(deadline, partition), _) = by_partition.iter().next().unwrap();
    let elsewhere = by_partition.values().nth(1).unwrap()[0];
    let params = UpgradeSectorQualityParams {
        extensions: vec![UpgradeSectorQuality {
            deadline,
            partition,
            sectors: make_bitfield(&[elsewhere]),
            new_expiration: None,
        }],
    };
    let res = h.upgrade_sector_quality(&rt, params, PowerPair::zero(), TokenAmount::zero());
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "can only upgrade active sectors",
        res,
    );
    rt.reset();

    // The same sector upgrades under its own partition, so the rejection was about the
    // declared location, not the sector's state.
    let elsewhere_record = legacy.iter().find(|s| s.sector_number == elsewhere).unwrap();
    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(elsewhere_record));
    h.upgrade_sector_quality(&rt, upgrade_params(&rt, elsewhere, None), power_delta, pledge_delta)
        .unwrap();
    assert_full_power_record(&h, &rt, elsewhere_record, &h.get_sector(&rt, elsewhere));
    h.check_state(&rt);
}

#[test]
fn rejects_expired_sector_awaiting_cron() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);

    // Past its expiration but not yet removed by the deadline cron: no longer upgradable.
    rt.set_epoch(legacy.expiration + 1);
    let res = h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        PowerPair::zero(),
        TokenAmount::zero(),
    );
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "cannot extend expiration for expired sector",
        res,
    );
    rt.reset();

    // At its expiration epoch it is still live and upgrades normally.
    rt.set_epoch(legacy.expiration);
    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();
    assert_full_power_record(&h, &rt, &legacy, &h.get_sector(&rt, legacy.sector_number));
    h.check_state(&rt);
}

#[test]
fn insufficient_balance_aborts_whole_message() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let (_, pledge_delta) = expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));

    // One atto short of the required top-up.
    let state: State = rt.get_state();
    rt.balance.replace(
        &state.initial_pledge + &state.locked_funds + &state.pre_commit_deposits + &pledge_delta
            - TokenAmount::from_atto(1),
    );

    let res = h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        PowerPair::zero(),
        TokenAmount::zero(),
    );
    expect_abort_contains_message(
        ExitCode::USR_INSUFFICIENT_FUNDS,
        "insufficient funds for aggregate initial pledge requirement",
        res,
    );
    rt.reset();

    // Nothing changed.
    let after = h.get_sector(&rt, legacy.sector_number);
    assert!(!after.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    assert_eq!(legacy.initial_pledge, after.initial_pledge);

    rt.balance.replace(BIG_BALANCE.clone());
    h.check_state(&rt);
}

#[test]
fn fee_debt_counts_against_available_balance() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let (_, pledge_delta) = expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));

    // Unlocked balance covers the top-up but not the fee debt as well, so the
    // debt-aware check must reject the upgrade.
    let debt = TokenAmount::from_whole(5);
    let mut state: State = rt.get_state();
    state.fee_debt = debt.clone();
    rt.balance.replace(
        &state.initial_pledge
            + &state.locked_funds
            + &state.pre_commit_deposits
            + &pledge_delta
            + &debt
            - TokenAmount::from_atto(1),
    );
    rt.replace_state(&state);

    let res = h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        PowerPair::zero(),
        TokenAmount::zero(),
    );
    expect_abort_contains_message(
        ExitCode::USR_INSUFFICIENT_FUNDS,
        "insufficient funds for aggregate initial pledge requirement",
        res,
    );
    rt.reset();

    rt.balance.replace(BIG_BALANCE.clone());
    h.check_state(&rt);
}

#[test]
fn repays_fee_debt_and_locks_pledge() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);

    let debt = TokenAmount::from_whole(5);
    let mut state: State = rt.get_state();
    state.fee_debt = debt.clone();
    rt.replace_state(&state);

    // The harness expects the fee-debt burn along with the upgrade effects.
    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        power_delta,
        pledge_delta,
    )
    .unwrap();

    let state: State = rt.get_state();
    assert!(state.fee_debt.is_zero());
    let upgraded = h.get_sector(&rt, legacy.sector_number);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(upgraded.initial_pledge, state.initial_pledge);
    h.check_state(&rt);
}

#[test]
fn rejects_unauthorized_caller() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let params = upgrade_params(&rt, legacy.sector_number, None);

    // The caller is checked before anything else, so no network queries are made.
    rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, Address::new_id(1234));
    rt.expect_validate_caller_addr(h.caller_addrs());
    let res = rt.call::<Actor>(
        Method::UpgradeSectorQuality as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    );
    expect_abort(ExitCode::USR_FORBIDDEN, res);
    h.check_state(&rt);
}

#[test]
fn rejects_terminated_sector() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);

    // Locked rewards ensure the termination fee is unlockable, as in the terminate tests.
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());
    let sector_power = qa_power_for_sector(h.sector_size, &legacy);
    let fault_fee = pledge_penalty_for_continued_fault(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &sector_power,
    );
    let termination_fee = pledge_penalty_for_termination(
        &legacy.initial_pledge,
        *rt.epoch.borrow() - legacy.activation,
        &fault_fee,
    );
    h.terminate_sectors(&rt, &make_bitfield(&[legacy.sector_number]), termination_fee);

    // A terminated sector stays in its partition's books until its record is cleaned up, but
    // it is no longer active.
    let res = h.upgrade_sector_quality(
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        PowerPair::zero(),
        TokenAmount::zero(),
    );
    expect_abort_contains_message(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "can only upgrade active sectors",
        res,
    );
    rt.reset();
    h.check_state(&rt);
}
