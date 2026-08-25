use export_macro::vm_test;
use fil_actor_miner::{
    ExpirationExtension2, ExtendSectorExpiration2Params, Method as MinerMethod, PowerPair,
    SectorOnChainInfoFlags, Sectors, State as MinerState, UpgradeSectorQuality,
    UpgradeSectorQualityParams, daily_proof_fee_adjust, initial_pledge_for_power,
    max_prove_commit_duration, qa_power_max,
};
use fil_actor_power::{State as PowerState, consensus_miner_min_power, set_claim};
use fil_actor_reward::State as RewardState;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{EPOCHS_IN_DAY, REWARD_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR};
use fvm_ipld_bitfield::BitField;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};
use num_traits::Zero;
use std::cmp::max;
use vm_api::VM;
use vm_api::trace::ExpectInvocation;
use vm_api::util::{DynBlockstore, apply_ok, get_state, mutate_state};

use crate::expects::Expect;
use crate::util::{
    PrecommitMetadata, advance_by_deadline_to_epoch, advance_by_deadline_to_index,
    advance_to_proving_deadline, assert_invariants, create_accounts, create_miner, cron_tick,
    miner_power, miner_precommit_one_sector_v2, miner_prove_sector,
    override_compute_unsealed_sector_cid, sector_info, submit_windowed_post,
};

/// Rewrites a freshly onboarded (10x) sector as a pre-FIP-0118 legacy CC sector:
/// `SIMPLE_QA_POWER` only, pledge and daily fee at their 1x rates. Moves the miner's
/// partition, deadline and pledge books with the record via the production
/// `replace_sectors`, and keeps the power actor's claim and totals consistent with the
/// rewritten books.
fn make_legacy_cc_sector(v: &dyn VM, maddr: &Address, sector_number: SectorNumber) {
    let store = DynBlockstore::wrap(v.blockstore());
    let policy = Policy::default();
    let mut power_delta = PowerPair::zero();
    let mut pledge_delta = TokenAmount::zero();

    mutate_state(v, maddr, |st: &mut MinerState| {
        let (dl_idx, p_idx) = st.find_sector(&store, sector_number).unwrap();
        let mut sectors = Sectors::load(&store, &st.sectors).unwrap();
        let old_sector = sectors.must_get(sector_number).unwrap();
        let sector_size = old_sector.seal_proof.sector_size().unwrap();

        let mut new_sector = old_sector.clone();
        new_sector.flags = SectorOnChainInfoFlags::SIMPLE_QA_POWER;
        new_sector.initial_pledge = old_sector.initial_pledge.div_floor(10);
        new_sector.daily_fee = daily_proof_fee_adjust(
            &old_sector.daily_fee,
            &qa_power_max(sector_size),
            &StoragePower::from(sector_size as u64),
        );

        let mut deadlines = st.load_deadlines(&store).unwrap();
        let mut deadline = deadlines.load_deadline(&store, dl_idx).unwrap();
        let mut partitions = deadline.partitions_amt(&store).unwrap();
        let mut partition = partitions.get(p_idx).unwrap().cloned().unwrap();
        let quant = st.quant_spec_for_deadline(&policy, dl_idx);
        let (partition_power_delta, partition_pledge_delta, partition_fee_delta) = partition
            .replace_sectors(
                &store,
                std::slice::from_ref(&old_sector),
                std::slice::from_ref(&new_sector),
                sector_size,
                quant,
            )
            .unwrap();
        deadline.live_power += &partition_power_delta;
        deadline.daily_fee += &partition_fee_delta;
        power_delta += &partition_power_delta;
        pledge_delta += &partition_pledge_delta;

        sectors.store(vec![new_sector]).unwrap();
        partitions.set(p_idx, partition).unwrap();
        deadline.partitions = partitions.flush().unwrap();
        deadlines.update_deadline(&policy, &store, dl_idx, &deadline).unwrap();
        st.sectors = sectors.amt.flush().unwrap();
        st.save_deadlines(&store, deadlines).unwrap();
        st.add_initial_pledge(&pledge_delta).unwrap();
    });

    mutate_state(v, &STORAGE_POWER_ACTOR_ADDR, |st: &mut PowerState| {
        let mut claims = st.load_claims(&store).unwrap();
        let mut claim = claims.get(maddr).unwrap().unwrap().clone();
        claim.raw_byte_power += &power_delta.raw;
        claim.quality_adj_power += &power_delta.qa;
        st.total_bytes_committed += &power_delta.raw;
        st.total_qa_bytes_committed += &power_delta.qa;
        // Only miners above the consensus minimum contribute to the power totals.
        let min_power = consensus_miner_min_power(&policy, claim.window_post_proof_type).unwrap();
        if claim.raw_byte_power >= min_power {
            st.total_raw_byte_power += &power_delta.raw;
            st.total_quality_adj_power += &power_delta.qa;
        }
        set_claim(&mut claims, maddr, claim).unwrap();
        st.save_claims(&mut claims).unwrap();
        st.total_pledge_collateral += &pledge_delta;
    });
}

/// FIP-0118: `UpgradeSectorQuality` upgrades a legacy CC sector to 10x QA power, locking the
/// pledge top-up. The upgraded sector is then skipped by a repeat call, and a plain extension
/// locks nothing more.
#[vm_test]
pub fn upgrade_sector_quality_upgrades_legacy_sector_test(v: &dyn VM) {
    override_compute_unsealed_sector_cid(v);
    let policy = Policy::default();
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_size = seal_proof.sector_size().unwrap();
    let (owner, worker) = (addrs[0], addrs[0]);
    let (maddr, miner_id) = {
        let (maddr, _) = create_miner(
            v,
            &owner,
            &worker,
            seal_proof.registered_window_post_proof().unwrap(),
            &TokenAmount::from_whole(8_000),
        );
        (maddr, maddr.id().unwrap())
    };
    let worker_id = worker.id().unwrap();

    v.set_epoch(200);

    // Onboard one CC sector and prove it, activating its (10x) power.
    let sector_number: SectorNumber = 100;
    let expiration = v.epoch()
        + policy.min_sector_expiration
        + max_prove_commit_duration(&policy, seal_proof).unwrap()
        + 1;
    miner_precommit_one_sector_v2(
        v,
        &worker,
        &maddr,
        seal_proof,
        sector_number,
        PrecommitMetadata::default(),
        true,
        expiration,
    );
    let prove_time = v.epoch() + policy.pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(v, &maddr, prove_time);
    miner_prove_sector(v, &worker, &maddr, sector_number, vec![]);
    cron_tick(v);

    let (deadline_info, partition_index) = advance_to_proving_deadline(v, &maddr, sector_number);
    let full_power =
        PowerPair { raw: StoragePower::from(sector_size as u64), qa: qa_power_max(sector_size) };
    submit_windowed_post(
        v,
        &worker,
        &maddr,
        deadline_info,
        partition_index,
        Some(full_power.clone()),
    );
    advance_by_deadline_to_index(
        v,
        &maddr,
        (deadline_info.index + 1) % policy.wpost_period_deadlines,
    );
    assert_eq!(full_power.qa, miner_power(v, &maddr).qa);

    // Fabricate the pre-FIP-0118 shape: 1x power, 1x pledge, 1x fee.
    make_legacy_cc_sector(v, &maddr, sector_number);
    let legacy = sector_info(v, &maddr, sector_number);
    assert!(!legacy.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    assert_eq!(full_power.raw, miner_power(v, &maddr).qa);

    let sectors = BitField::try_from_bits([sector_number]).unwrap();
    let upgrade_params = UpgradeSectorQualityParams {
        extensions: vec![UpgradeSectorQuality {
            deadline: deadline_info.index,
            partition: partition_index,
            sectors: sectors.clone(),
            new_expiration: None,
        }],
    };

    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::UpgradeSectorQuality as u64,
        Some(upgrade_params),
    );

    // The record reflects the upgrade, in place.
    let upgraded = sector_info(v, &maddr, sector_number);
    assert!(
        upgraded.flags.contains(
            SectorOnChainInfoFlags::FULL_QA_POWER | SectorOnChainInfoFlags::SIMPLE_QA_POWER
        )
    );
    assert_eq!(legacy.expiration, upgraded.expiration);
    assert_eq!(legacy.power_base_epoch, upgraded.power_base_epoch);
    assert_eq!(
        daily_proof_fee_adjust(
            &legacy.daily_fee,
            &StoragePower::from(sector_size as u64),
            &qa_power_max(sector_size),
        ),
        upgraded.daily_fee
    );
    let reward_state: RewardState = get_state(v, &REWARD_ACTOR_ADDR).unwrap();
    let power_state: PowerState = get_state(v, &STORAGE_POWER_ACTOR_ADDR).unwrap();
    let full_power_pledge = initial_pledge_for_power(
        &qa_power_max(sector_size),
        &reward_state.this_epoch_baseline_power,
        &reward_state.this_epoch_reward_smoothed,
        &power_state.this_epoch_qa_power_smoothed,
        &v.circulating_supply(),
        v.epoch() - power_state.ramp_start_epoch,
        power_state.ramp_duration_epochs,
    );
    assert_eq!(max(legacy.initial_pledge.clone(), full_power_pledge), upgraded.initial_pledge);

    // The power actor was told about the QA lift and the pledge top-up.
    let pledge_delta = &upgraded.initial_pledge - &legacy.initial_pledge;
    assert!(pledge_delta.is_positive());
    ExpectInvocation {
        from: worker_id,
        to: maddr,
        method: MinerMethod::UpgradeSectorQuality as u64,
        subinvocs: Some(vec![
            Expect::reward_this_epoch(miner_id),
            Expect::power_current_total(miner_id),
            Expect::power_update_claim(
                miner_id,
                PowerPair { raw: BigInt::zero(), qa: &full_power.qa - &full_power.raw },
            ),
            Expect::power_update_pledge(miner_id, Some(pledge_delta)),
        ]),
        events: Some(vec![]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
    assert_eq!(full_power.qa, miner_power(v, &maddr).qa);

    // Already at full power, the sector is not eligible: a repeat call — even one asking for
    // an extension — skips it, changing nothing and notifying nobody.
    let new_expiration = upgraded.expiration + 42 * EPOCHS_IN_DAY;
    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::UpgradeSectorQuality as u64,
        Some(UpgradeSectorQualityParams {
            extensions: vec![UpgradeSectorQuality {
                deadline: deadline_info.index,
                partition: partition_index,
                sectors: sectors.clone(),
                new_expiration: Some(new_expiration),
            }],
        }),
    );
    assert_eq!(upgraded, sector_info(v, &maddr, sector_number));
    ExpectInvocation {
        from: worker_id,
        to: maddr,
        method: MinerMethod::UpgradeSectorQuality as u64,
        subinvocs: Some(vec![
            Expect::reward_this_epoch(miner_id),
            Expect::power_current_total(miner_id),
        ]),
        events: Some(vec![]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // A plain extension moves the expiration and locks nothing more: flag, pledge and fee ride
    // along, and no power or pledge notification is sent at all.
    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::ExtendSectorExpiration2 as u64,
        Some(ExtendSectorExpiration2Params {
            extensions: vec![ExpirationExtension2 {
                deadline: deadline_info.index,
                partition: partition_index,
                sectors,
                sectors_with_claims: vec![],
                new_expiration,
            }],
        }),
    );
    let extended = sector_info(v, &maddr, sector_number);
    assert_eq!(new_expiration, extended.expiration);
    assert_eq!(v.epoch(), extended.power_base_epoch);
    assert_eq!(upgraded.flags, extended.flags);
    assert_eq!(upgraded.initial_pledge, extended.initial_pledge);
    assert_eq!(upgraded.daily_fee, extended.daily_fee);
    ExpectInvocation {
        from: worker_id,
        to: maddr,
        method: MinerMethod::ExtendSectorExpiration2 as u64,
        subinvocs: Some(vec![]),
        events: Some(vec![]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());

    // The sector keeps proving at its upgraded power.
    let (deadline_info, partition_index) = advance_to_proving_deadline(v, &maddr, sector_number);
    submit_windowed_post(v, &worker, &maddr, deadline_info, partition_index, None);
    advance_by_deadline_to_index(
        v,
        &maddr,
        (deadline_info.index + 1) % policy.wpost_period_deadlines,
    );
    assert_eq!(full_power.qa, miner_power(v, &maddr).qa);

    assert_invariants(v, &policy, None);
}
