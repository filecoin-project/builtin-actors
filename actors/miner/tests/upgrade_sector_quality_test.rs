use fil_actor_miner::{
    Actor, ExpirationExtension2, ExtendSectorExpiration2Params, Method, PoStPartition, PowerPair,
    SectorOnChainInfo, SectorOnChainInfoFlags, State, UpgradeSectorQuality,
    UpgradeSectorQualityParams, daily_proof_fee, daily_proof_fee_adjust,
    pledge_penalty_for_termination, power_for_sectors, qa_power_for_sector, qa_power_max,
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
use fvm_shared::sector::{RegisteredSealProof, SectorNumber};

use num_traits::Zero;
use std::cmp::max;
use std::collections::BTreeMap;
use std::ops::Neg;

mod util;
use util::*;

fn setup() -> (ActorHarness, MockRuntime) {
    let mut h = ActorHarness::new(100);
    h.set_proof_type(RegisteredSealProof::StackedDRG512MiBV1);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    rt.set_epoch(1);
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
        h.commit_and_prove_sectors(rt, count, DEFAULT_SECTOR_EXPIRATION, Vec::new(), true);
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

fn declaration(
    deadline: u64,
    partition: u64,
    sectors: &[SectorNumber],
    new_expiration: Option<ChainEpoch>,
) -> UpgradeSectorQuality {
    UpgradeSectorQuality { deadline, partition, sectors: make_bitfield(sectors), new_expiration }
}

/// One upgrade-only declaration per (deadline, partition) home of the sectors.
fn upgrade_only_declarations(
    rt: &MockRuntime,
    sectors: &[SectorOnChainInfo],
) -> Vec<UpgradeSectorQuality> {
    group_by_partition(rt, sectors)
        .into_iter()
        .map(|((deadline, partition), sectors)| declaration(deadline, partition, &sectors, None))
        .collect()
}

fn upgrade_params(
    rt: &MockRuntime,
    sector_number: SectorNumber,
    new_expiration: Option<ChainEpoch>,
) -> UpgradeSectorQualityParams {
    let (deadline, partition) = sector_location(rt, sector_number);
    UpgradeSectorQualityParams {
        upgrades: vec![declaration(deadline, partition, &[sector_number], new_expiration)],
    }
}

/// Power and pledge deltas expected from upgrading these sectors to full power: power rises to
/// the maximum, pledge to max(old, 10x requirement) — never down.
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

/// Upgrades one sector, expecting exactly the power and pledge deltas its record implies, and
/// returns the new record.
fn upgrade_one(
    h: &ActorHarness,
    rt: &MockRuntime,
    legacy: &SectorOnChainInfo,
    new_expiration: Option<ChainEpoch>,
) -> SectorOnChainInfo {
    let (power_delta, pledge_delta) = expected_upgrade_deltas(h, rt, std::slice::from_ref(legacy));
    h.upgrade_sector_quality(
        rt,
        upgrade_params(rt, legacy.sector_number, new_expiration),
        power_delta,
        pledge_delta,
    )
    .unwrap();
    h.get_sector(rt, legacy.sector_number)
}

/// Calls `UpgradeSectorQuality` on a sector already at full power, which may move its
/// expiration but never its power or pledge, and returns its record.
fn call_at_full_power(
    h: &ActorHarness,
    rt: &MockRuntime,
    sector: &SectorOnChainInfo,
    new_expiration: Option<ChainEpoch>,
) -> SectorOnChainInfo {
    h.upgrade_sector_quality(
        rt,
        upgrade_params(rt, sector.sector_number, new_expiration),
        PowerPair::zero(),
        TokenAmount::zero(),
    )
    .unwrap();
    h.get_sector(rt, sector.sector_number)
}

/// Calls `UpgradeSectorQuality` expecting it to abort with `code` and `message`.
fn expect_upgrade_abort(
    h: &ActorHarness,
    rt: &MockRuntime,
    params: UpgradeSectorQualityParams,
    code: ExitCode,
    message: &str,
) {
    let res = h.upgrade_sector_quality(rt, params, PowerPair::zero(), TokenAmount::zero());
    expect_abort_contains_message(code, message, res);
    rt.reset();
}

/// The fee `TerminateSectors` charges for `sector` now: the pledge-and-age formula, floored by
/// the continued-fault fee at the sector's current power.
fn termination_fee(h: &ActorHarness, rt: &MockRuntime, sector: &SectorOnChainInfo) -> TokenAmount {
    let fault_fee = h.continued_fault_penalty(std::slice::from_ref(sector));
    pledge_penalty_for_termination(
        &sector.initial_pledge,
        *rt.epoch.borrow() - sector.activation,
        &fault_fee,
    )
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
    let upgraded = upgrade_one(&h, &rt, &legacy, None);

    // Only the flags, pledge and fee changed on the record.
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
    let new_expiration = legacy.expiration + 42 * EPOCHS_IN_DAY;

    let upgraded = upgrade_one(&h, &rt, &legacy, Some(new_expiration));
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(new_expiration, upgraded.expiration);
    assert_eq!(*rt.epoch.borrow(), upgraded.power_base_epoch);
    // check_state requires the partition and deadline queues to hold the sector at its new
    // expiration.
    h.check_state(&rt);
}

#[test]
fn repeated_upgrade_is_a_no_op() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let upgraded = upgrade_one(&h, &rt, &legacy, None);
    let state_root = *rt.state.borrow();

    // A sector at full power has nothing to upgrade: an in-place repeat moves no power,
    // pledge or fee, and state stays byte-identical.
    assert_eq!(upgraded, call_at_full_power(&h, &rt, &upgraded, None));
    assert_eq!(state_root, *rt.state.borrow());
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
    let after_first = upgrade_one(&h, &rt, &legacy, None);
    assert_eq!(legacy.initial_pledge, after_first.initial_pledge);

    // Conditions recover, so the full-power requirement now exceeds what the sector holds.
    // Already at full power, a repeat locks nothing: in place it changes nothing, with a
    // new expiration it only extends.
    rt.set_circulating_supply(normal_supply);
    h.epoch_reward_smooth = normal_reward;
    let raised_requirement = h.initial_pledge_for_power(&rt, &qa_power_max(h.sector_size));
    assert!(raised_requirement > after_first.initial_pledge);
    let new_expiration = after_first.expiration + 42 * EPOCHS_IN_DAY;
    for repeat in [None, Some(new_expiration)] {
        let sector = call_at_full_power(&h, &rt, &after_first, repeat);
        assert_eq!(repeat.unwrap_or(after_first.expiration), sector.expiration);
        assert_eq!(after_first.flags, sector.flags);
        assert_eq!(after_first.initial_pledge, sector.initial_pledge);
        assert_eq!(after_first.daily_fee, sector.daily_fee);
    }
    assert_eq!(after_first.initial_pledge, rt.get_state::<State>().initial_pledge);
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

    // Full power, with pledge and fee scaled up from 5.5x; the weights stay as they were, now
    // masked by the flag.
    let upgraded = upgrade_one(&h, &rt, &legacy, None);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(legacy.verified_deal_weight, upgraded.verified_deal_weight);
    assert_eq!(legacy.power_base_epoch, upgraded.power_base_epoch);
    h.check_state(&rt);
}

#[test]
fn upgrades_sector_with_unverified_data_keeping_weights() {
    let (mut h, rt) = setup();
    // A legacy sector half-full of unverified deal data: deal weight has no power effect, so
    // it is a 1x sector that happens to carry a deal weight.
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let half_space = BigInt::from(h.sector_size as u64 / 2);
    h.rewrite_sectors(&rt, &[legacy.sector_number], |s| {
        s.deal_weight = &half_space * (s.expiration - s.power_base_epoch)
    });
    let legacy = h.get_sector(&rt, legacy.sector_number);
    assert_eq!(BigInt::from(h.sector_size as u64), qa_power_for_sector(h.sector_size, &legacy));

    // The same full 9x jump as for a CC sector, and the deal weight survives untouched.
    let upgraded = upgrade_one(&h, &rt, &legacy, None);
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

    // Same weight math as ExtendSectorExpiration2: the verified space is restated over the new
    // duration. Power comes from the flag, so the restated weight is not what raised it.
    let upgraded = upgrade_one(&h, &rt, &legacy, Some(new_expiration));
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
fn already_full_power_by_weights_is_not_upgraded() {
    let (mut h, rt) = setup();
    // Already 10x through its weights alone, so there is nothing to raise.
    let full = h.sector_size as u64;
    let legacy =
        commit_legacy_sectors(&mut h, &rt, 1, SectorOnChainInfoFlags::SIMPLE_QA_POWER, full)
            .remove(0);
    assert_eq!(qa_power_max(h.sector_size), qa_power_for_sector(h.sector_size, &legacy));

    // In place, the sector is skipped — not even flagged — leaving state byte-identical.
    let state_root = *rt.state.borrow();
    assert_eq!(legacy, call_at_full_power(&h, &rt, &legacy, None));
    assert_eq!(state_root, *rt.state.borrow());

    // With a new expiration it is extended as ExtendSectorExpiration2 would: still
    // unflagged, its verified weight restated so it stays at 10x, pledge and fee untouched.
    let new_expiration = legacy.expiration + 42 * EPOCHS_IN_DAY;
    let extended = call_at_full_power(&h, &rt, &legacy, Some(new_expiration));
    assert_eq!(legacy.flags, extended.flags);
    assert_eq!(new_expiration, extended.expiration);
    assert_eq!(*rt.epoch.borrow(), extended.power_base_epoch);
    assert_eq!(qa_power_max(h.sector_size), qa_power_for_sector(h.sector_size, &extended));
    assert_eq!(legacy.initial_pledge, extended.initial_pledge);
    assert_eq!(legacy.daily_fee, extended.daily_fee);
    h.check_state(&rt);
}

#[test]
fn keeps_higher_existing_pledge_without_topping_up() {
    let (mut h, rt) = setup();
    // The sector holds twice today's 10x requirement (it onboarded when pledge ran higher).
    // The rule is max(old, new): nothing extra to lock, and the pledge is never lowered.
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let pledge_10x = h.initial_pledge_for_power(&rt, &qa_power_max(h.sector_size));
    let high_pledge = &pledge_10x * 2;
    h.rewrite_sectors(&rt, &[legacy.sector_number], |s| s.initial_pledge = high_pledge.clone());
    let legacy = h.get_sector(&rt, legacy.sector_number);
    let state_before: State = rt.get_state();

    // Power and fee still move to the 10x level; the pledge stays put.
    let upgraded = upgrade_one(&h, &rt, &legacy, None);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(high_pledge, upgraded.initial_pledge);
    assert_eq!(state_before.initial_pledge, rt.get_state::<State>().initial_pledge);
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
    let upgrades = vec![
        declaration(deadline, partition, &[numbers[0]], None),
        declaration(
            deadline,
            partition,
            &[numbers[1]],
            Some(legacy[1].expiration + 42 * EPOCHS_IN_DAY),
        ),
    ];
    let (power_delta, pledge_delta) = expected_upgrade_deltas(&h, &rt, &legacy);
    h.upgrade_sector_quality(
        &rt,
        UpgradeSectorQualityParams { upgrades },
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
    let numbers = [legacy[0].sector_number, legacy[1].sector_number];
    let (power_delta, pledge_delta) = expected_upgrade_deltas(&h, &rt, &legacy);
    h.upgrade_sector_quality(
        &rt,
        UpgradeSectorQualityParams {
            upgrades: vec![declaration(deadline, partition, &numbers, None)],
        },
        power_delta,
        pledge_delta,
    )
    .unwrap();

    for sector in &legacy {
        let upgraded = h.get_sector(&rt, sector.sector_number);
        assert_full_power_record(&h, &rt, sector, &upgraded);
        assert_eq!(sector.expiration, upgraded.expiration);
    }
    h.check_state(&rt);
}

#[test]
fn extension_extends_but_does_not_upgrade_full_power_sectors() {
    let (mut h, rt) = setup();
    let sectors = commit_proven_sectors(&mut h, &rt, 2);
    let (deadline, partition) = sector_location(&rt, sectors[0].sector_number);
    assert_eq!((deadline, partition), sector_location(&rt, sectors[1].sector_number));

    // One sector is made legacy; its partition-mate keeps its as-committed full power.
    let legacy =
        make_legacy(&h, &rt, &sectors[..1], SectorOnChainInfoFlags::SIMPLE_QA_POWER, 0).remove(0);
    let full = h.get_sector(&rt, sectors[1].sector_number);
    assert!(full.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));

    // One declaration extends both. The legacy sector is upgraded and extended; the
    // full-power one is only extended, as ExtendSectorExpiration2 would, so the batch locks
    // just the legacy sector's top-up.
    let new_expiration = legacy.expiration + 42 * EPOCHS_IN_DAY;
    let both = [legacy.sector_number, full.sector_number];
    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(
        &rt,
        UpgradeSectorQualityParams {
            upgrades: vec![declaration(deadline, partition, &both, Some(new_expiration))],
        },
        power_delta,
        pledge_delta,
    )
    .unwrap();

    let upgraded = h.get_sector(&rt, legacy.sector_number);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(new_expiration, upgraded.expiration);
    let extended = h.get_sector(&rt, full.sector_number);
    assert_eq!(new_expiration, extended.expiration);
    assert_eq!(*rt.epoch.borrow(), extended.power_base_epoch);
    assert_eq!(full.flags, extended.flags);
    assert_eq!(full.initial_pledge, extended.initial_pledge);
    assert_eq!(full.daily_fee, extended.daily_fee);
    h.check_state(&rt);
}

#[test]
fn mixed_batch_upgrades_extends_and_requeues_across_epochs() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_sectors(&mut h, &rt, 4, SectorOnChainInfoFlags::SIMPLE_QA_POWER, 0);

    // One partition holding two sectors, and one other partition.
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

    // Two different epochs inside one partition, an in-place upgrade in the other partition,
    // and that partition's second sector left out of the batch entirely.
    let base = max(by_number[&pair[0]].expiration, by_number[&pair[1]].expiration);
    let epoch_one = base + 30 * EPOCHS_IN_DAY;
    let epoch_two = base + 60 * EPOCHS_IN_DAY;
    let upgrades = vec![
        declaration(deadline, partition, &[pair[0]], Some(epoch_one)),
        declaration(deadline, partition, &[pair[1]], Some(epoch_two)),
        declaration(other_deadline, other_partition, &[other[0]], None),
    ];
    let declared =
        [by_number[&pair[0]].clone(), by_number[&pair[1]].clone(), by_number[&other[0]].clone()];
    let (power_delta, pledge_delta) = expected_upgrade_deltas(&h, &rt, &declared);
    h.upgrade_sector_quality(
        &rt,
        UpgradeSectorQualityParams { upgrades },
        power_delta,
        pledge_delta,
    )
    .unwrap();

    // The extended sectors landed on their own epochs, the in-place one kept its schedule and
    // the undeclared one is untouched; check_state verifies the queues and books against the
    // records.
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
fn later_declaration_extends_an_upgraded_sector_again() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let (deadline, partition) = sector_location(&rt, legacy.sector_number);
    let first = legacy.expiration + 30 * EPOCHS_IN_DAY;
    let second = legacy.expiration + 60 * EPOCHS_IN_DAY;
    let declare_twice = |expirations: [ChainEpoch; 2]| UpgradeSectorQualityParams {
        upgrades: expirations
            .iter()
            .map(|&e| declaration(deadline, partition, &[legacy.sector_number], Some(e)))
            .collect(),
    };

    // Declarations apply in order: the first upgrades and extends the sector, the second
    // finds it at full power and extends it again, which cannot shorten it...
    expect_upgrade_abort(
        &h,
        &rt,
        declare_twice([second, first]),
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!("cannot reduce sector {} expiration", legacy.sector_number),
    );

    // ...so the last expiration stands, and the sector is charged its top-up once.
    let (power_delta, pledge_delta) =
        expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
    h.upgrade_sector_quality(&rt, declare_twice([first, second]), power_delta, pledge_delta)
        .unwrap();
    let upgraded = h.get_sector(&rt, legacy.sector_number);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);
    assert_eq!(second, upgraded.expiration);
    h.check_state(&rt);
}

#[test]
fn deadline_cron_charges_the_upgraded_fee() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let upgraded = upgrade_one(&h, &rt, &legacy, None);
    assert!(upgraded.daily_fee > legacy.daily_fee);

    // The deadline's fee book carries the upgraded fee, and the next period's cron charges
    // that book: the harness expects the burn it derives from the record, to the atto.
    let (deadline_index, _) = sector_location(&rt, legacy.sector_number);
    assert_eq!(upgraded.daily_fee, h.get_deadline(&rt, deadline_index).daily_fee);
    h.advance_and_submit_posts(&rt, std::slice::from_ref(&upgraded));
    h.check_state(&rt);
}

#[test]
fn deadline_cron_releases_upgraded_power_and_pledge_at_expiration() {
    // A one-period minimum lifetime keeps the sector's whole life to a few proving periods.
    let (mut h, mut rt) = setup();
    rt.policy.min_sector_expiration = rt.policy.wpost_proving_period;
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
    let upgraded = upgrade_one(&h, &rt, &legacy, None);
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
    assert!(rt.get_state::<State>().initial_pledge.is_zero());
    h.check_state(&rt);
}

#[test]
fn terminates_upgraded_sector_at_upgraded_pledge_and_power() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let upgraded = upgrade_one(&h, &rt, &legacy, None);
    // Locked rewards ensure the fee is unlockable, as in the terminate tests.
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());

    // The termination fee's fault-fee floor runs on the upgraded (10x) power, and the released
    // pledge is the topped-up one.
    assert_eq!(qa_power_max(h.sector_size), qa_power_for_sector(h.sector_size, &upgraded));
    let expected_fee = termination_fee(&h, &rt, &upgraded);
    let (power_removed, pledge_removed) =
        h.terminate_sectors(&rt, &make_bitfield(&[upgraded.sector_number]), expected_fee.clone());
    assert_eq!(
        power_for_sectors(h.sector_size, std::slice::from_ref(&upgraded)).neg(),
        power_removed
    );
    assert_eq!(-(expected_fee + &upgraded.initial_pledge), pledge_removed);
    assert!(rt.get_state::<State>().initial_pledge.is_zero());
    h.check_state(&rt);
}

#[test]
fn extend2_after_upgrade_keeps_full_power_and_pledge() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let upgraded = upgrade_one(&h, &rt, &legacy, None);

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
fn extension_of_full_power_sector_matches_extend_sector_expiration2() {
    let (mut h, rt) = setup();
    // Two identical half-verified legacy sectors, upgraded in place, so both carry a
    // verified weight to restate.
    let half = h.sector_size as u64 / 2;
    let legacy =
        commit_legacy_sectors(&mut h, &rt, 2, SectorOnChainInfoFlags::SIMPLE_QA_POWER, half);
    let (power_delta, pledge_delta) = expected_upgrade_deltas(&h, &rt, &legacy);
    h.upgrade_sector_quality(
        &rt,
        UpgradeSectorQualityParams { upgrades: upgrade_only_declarations(&rt, &legacy) },
        power_delta,
        pledge_delta,
    )
    .unwrap();

    // Extend one through UpgradeSectorQuality and the other through ExtendSectorExpiration2.
    let new_expiration = legacy[0].expiration + 42 * EPOCHS_IN_DAY;
    let upgraded = h.get_sector(&rt, legacy[0].sector_number);
    let by_upgrade = call_at_full_power(&h, &rt, &upgraded, Some(new_expiration));
    let (deadline, partition) = sector_location(&rt, legacy[1].sector_number);
    h.extend_sectors2(
        &rt,
        ExtendSectorExpiration2Params {
            extensions: vec![ExpirationExtension2 {
                deadline,
                partition,
                sectors: make_bitfield(&[legacy[1].sector_number]),
                sectors_with_claims: vec![],
                new_expiration,
            }],
        },
    )
    .unwrap();

    // The records agree in everything but the sector's identity.
    let mut by_extend2 = h.get_sector(&rt, legacy[1].sector_number);
    by_extend2.sector_number = by_upgrade.sector_number;
    by_extend2.sealed_cid = by_upgrade.sealed_cid;
    assert_eq!(by_extend2, by_upgrade);
    h.check_state(&rt);
}

#[test]
fn withdraw_after_upgrade_leaves_the_new_pledge_locked() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    upgrade_one(&h, &rt, &legacy, None);

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
    h.rewrite_sectors(&rt, &[legacy.sector_number], |s| s.daily_fee = TokenAmount::zero());
    let legacy = h.get_sector(&rt, legacy.sector_number);

    let upgraded = upgrade_one(&h, &rt, &legacy, None);
    let expected_fee =
        daily_proof_fee(rt.policy(), &rt.total_fil_circ_supply(), &qa_power_max(h.sector_size));
    assert!(expected_fee.is_positive());
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
    expect_upgrade_abort(
        &h,
        &rt,
        UpgradeSectorQualityParams { upgrades: vec![] },
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "no extension declarations",
    );
    h.check_state(&rt);
}

#[test]
fn rejects_empty_sector_selection() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let (deadline, partition) = sector_location(&rt, legacy.sector_number);

    // The empty-selection check fires before the expiration checks.
    for new_expiration in [None, Some(i64::MAX)] {
        expect_upgrade_abort(
            &h,
            &rt,
            UpgradeSectorQualityParams {
                upgrades: vec![declaration(deadline, partition, &[], new_expiration)],
            },
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "no sectors selected",
        );
    }
    h.check_state(&rt);
}

#[test]
fn rejects_out_of_range_expirations() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let curr_epoch = *rt.epoch.borrow();

    // The first four are rejected before the network queries; reducing is a per-sector check.
    let cases = [
        (0, "must be after current epoch".to_string()),
        (-1, "must be after current epoch".to_string()),
        (curr_epoch, "must be after current epoch".to_string()),
        (
            curr_epoch + rt.policy().max_sector_expiration_extension + 1,
            "cannot be more than".to_string(),
        ),
        (
            legacy.expiration - 1,
            format!("cannot reduce sector {} expiration", legacy.sector_number),
        ),
    ];
    for (new_expiration, message) in cases {
        expect_upgrade_abort(
            &h,
            &rt,
            upgrade_params(&rt, legacy.sector_number, Some(new_expiration)),
            ExitCode::USR_ILLEGAL_ARGUMENT,
            &message,
        );
    }

    // The per-sector check also guards the extension of a sector already at full power.
    let upgraded = upgrade_one(&h, &rt, &legacy, None);
    expect_upgrade_abort(
        &h,
        &rt,
        upgrade_params(&rt, upgraded.sector_number, Some(upgraded.expiration - 1)),
        ExitCode::USR_ILLEGAL_ARGUMENT,
        &format!("cannot reduce sector {} expiration", legacy.sector_number),
    );
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
        expect_upgrade_abort(
            &h,
            &rt,
            UpgradeSectorQualityParams {
                upgrades: vec![declaration(deadline, partition, &[sector_number], None)],
            },
            code,
            message,
        );
    }
    h.check_state(&rt);
}

#[test]
fn faulty_declaration_aborts_the_whole_batch() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_sectors(&mut h, &rt, 3, SectorOnChainInfoFlags::SIMPLE_QA_POWER, 0);
    let faulty = legacy[2].clone();
    h.declare_faults(&rt, std::slice::from_ref(&faulty));

    // Good declarations first, the faulty sector's last: a faulty sector anywhere in the batch
    // fails the whole message.
    let mut declarations = upgrade_only_declarations(&rt, &legacy);
    declarations.sort_by_key(|d| d.sectors.get(faulty.sector_number));
    assert!(declarations.len() >= 2, "need the faulty sector in its own declaration");
    expect_upgrade_abort(
        &h,
        &rt,
        UpgradeSectorQualityParams { upgrades: declarations },
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "is not active in",
    );
    h.check_state(&rt);
}

#[test]
fn rejects_unproven_sector() {
    let (mut h, rt) = setup();
    h.construct_and_verify(&rt);
    // Committed but not yet proven in a WindowPoSt, so not yet active.
    let sector =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION, Vec::new(), true).remove(0);
    expect_upgrade_abort(
        &h,
        &rt,
        upgrade_params(&rt, sector.sector_number, None),
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "is not active in",
    );
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
    expect_upgrade_abort(
        &h,
        &rt,
        UpgradeSectorQualityParams {
            upgrades: vec![declaration(deadline, partition, &[elsewhere], None)],
        },
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "is not active in",
    );

    // The same sector upgrades under its own partition, so the rejection was about the
    // declared location, not the sector's state.
    let elsewhere_record = legacy.iter().find(|s| s.sector_number == elsewhere).unwrap();
    let upgraded = upgrade_one(&h, &rt, elsewhere_record, None);
    assert_full_power_record(&h, &rt, elsewhere_record, &upgraded);
    h.check_state(&rt);
}

#[test]
fn rejects_terminated_sector() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    // Locked rewards ensure the termination fee is unlockable, as in the terminate tests.
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());
    h.terminate_sectors(
        &rt,
        &make_bitfield(&[legacy.sector_number]),
        termination_fee(&h, &rt, &legacy),
    );

    // A terminated sector stays in its partition's books until its record is cleaned up, but
    // it is no longer active.
    expect_upgrade_abort(
        &h,
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        ExitCode::USR_ILLEGAL_ARGUMENT,
        "is not active in",
    );
    h.check_state(&rt);
}

#[test]
fn rejects_expired_sector_awaiting_cron() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);

    // Past its expiration but not yet removed by the deadline cron: no longer upgradable.
    rt.set_epoch(legacy.expiration + 1);
    expect_upgrade_abort(
        &h,
        &rt,
        upgrade_params(&rt, legacy.sector_number, None),
        ExitCode::USR_FORBIDDEN,
        "cannot extend expiration for expired sector",
    );

    // At its expiration epoch it is still live and upgrades normally.
    rt.set_epoch(legacy.expiration);
    let upgraded = upgrade_one(&h, &rt, &legacy, None);
    assert_full_power_record(&h, &rt, &legacy, &upgraded);

    // Expired at full power, an extension is refused as ExtendSectorExpiration2 refuses it,
    // while an in-place repeat has nothing to do and nothing to reject.
    rt.set_epoch(legacy.expiration + 1);
    expect_upgrade_abort(
        &h,
        &rt,
        upgrade_params(&rt, upgraded.sector_number, Some(upgraded.expiration + EPOCHS_IN_DAY)),
        ExitCode::USR_FORBIDDEN,
        "cannot extend expiration for expired sector",
    );
    assert_eq!(upgraded, call_at_full_power(&h, &rt, &upgraded, None));
    h.check_state(&rt);
}

#[test]
fn insufficient_available_balance_aborts_whole_message() {
    // Available balance is the unlocked balance less fee debt: one atto short of the top-up,
    // with or without debt, is rejected before anything is locked.
    for debt in [TokenAmount::zero(), TokenAmount::from_whole(5)] {
        let (mut h, rt) = setup();
        let legacy = commit_legacy_cc_sector(&mut h, &rt);
        let (_, pledge_delta) = expected_upgrade_deltas(&h, &rt, std::slice::from_ref(&legacy));
        let mut state: State = rt.get_state();
        state.fee_debt = debt.clone();
        rt.replace_state(&state);
        rt.balance.replace(
            &state.initial_pledge
                + &state.locked_funds
                + &state.pre_commit_deposits
                + &pledge_delta
                + &debt
                - TokenAmount::from_atto(1),
        );

        expect_upgrade_abort(
            &h,
            &rt,
            upgrade_params(&rt, legacy.sector_number, None),
            ExitCode::USR_INSUFFICIENT_FUNDS,
            "insufficient funds for aggregate initial pledge requirement",
        );
        rt.balance.replace(BIG_BALANCE.clone());
        h.check_state(&rt);
    }
}

#[test]
fn repays_fee_debt_and_locks_pledge() {
    let (mut h, rt) = setup();
    let legacy = commit_legacy_cc_sector(&mut h, &rt);
    let mut state: State = rt.get_state();
    state.fee_debt = TokenAmount::from_whole(5);
    rt.replace_state(&state);

    // The harness expects the fee-debt burn along with the upgrade effects.
    let upgraded = upgrade_one(&h, &rt, &legacy, None);
    let state: State = rt.get_state();
    assert!(state.fee_debt.is_zero());
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
