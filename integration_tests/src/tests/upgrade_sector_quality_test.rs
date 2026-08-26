use export_macro::vm_test;
use fil_actor_miner::{
    Method as MinerMethod, PowerPair, SectorOnChainInfoFlags, Sectors, State as MinerState,
    UpgradeSectorQuality, UpgradeSectorQualityParams, daily_proof_fee_adjust, qa_power_max,
};
use fil_actor_power::{State as PowerState, consensus_miner_min_power, set_claim};
use fil_actors_runtime::STORAGE_POWER_ACTOR_ADDR;
use fil_actors_runtime::runtime::Policy;
use fvm_ipld_bitfield::BitField;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, StoragePower};
use num_traits::Zero;
use std::cmp::max;
use vm_api::VM;
use vm_api::trace::ExpectInvocation;
use vm_api::util::{DynBlockstore, apply_ok, mutate_state};

use crate::expects::Expect;
use crate::tests::{create_sector, current_initial_pledge};
use crate::util::{
    advance_by_deadline_to_index, advance_to_proving_deadline, assert_invariants, create_accounts,
    create_miner, miner_power, override_compute_unsealed_sector_cid, sector_info,
    submit_windowed_post,
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
/// pledge top-up. A repeat call upgrades nothing more: in place it is a no-op, with a new
/// expiration it only extends.
#[vm_test]
pub fn upgrade_sector_quality_upgrades_legacy_sector_test(v: &dyn VM) {
    override_compute_unsealed_sector_cid(v);
    let policy = Policy::default();
    let addrs = create_accounts(v, 1, &TokenAmount::from_whole(10_000));
    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let sector_size = seal_proof.sector_size().unwrap();
    let (owner, worker) = (addrs[0], addrs[0]);
    let (maddr, _) = create_miner(
        v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(8_000),
    );
    let miner_id = maddr.id().unwrap();
    let worker_id = worker.id().unwrap();
    v.set_epoch(200);

    // Onboard and prove one CC sector, activating its (10x) power, then rewrite it into the
    // pre-FIP-0118 shape: 1x power, 1x pledge, 1x fee.
    let sector_number: SectorNumber = 100;
    let (deadline, partition) = create_sector(v, worker, maddr, sector_number, seal_proof);
    let raw_power = StoragePower::from(sector_size as u64);
    let full_qa_power = qa_power_max(sector_size);
    assert_eq!(full_qa_power, miner_power(v, &maddr).qa);
    make_legacy_cc_sector(v, &maddr, sector_number);
    let legacy = sector_info(v, &maddr, sector_number);
    assert!(!legacy.flags.contains(SectorOnChainInfoFlags::FULL_QA_POWER));
    assert_eq!(raw_power, miner_power(v, &maddr).qa);

    // Upgrade in place.
    let sectors = BitField::try_from_bits([sector_number]).unwrap();
    apply_ok(
        v,
        &worker,
        &maddr,
        &TokenAmount::zero(),
        MinerMethod::UpgradeSectorQuality as u64,
        Some(UpgradeSectorQualityParams {
            extensions: vec![UpgradeSectorQuality {
                deadline,
                partition,
                sectors: sectors.clone(),
                new_expiration: None,
            }],
        }),
    );
    let upgraded = sector_info(v, &maddr, sector_number);
    assert!(
        upgraded.flags.contains(
            SectorOnChainInfoFlags::FULL_QA_POWER | SectorOnChainInfoFlags::SIMPLE_QA_POWER
        )
    );
    assert_eq!(legacy.expiration, upgraded.expiration);
    assert_eq!(legacy.power_base_epoch, upgraded.power_base_epoch);
    assert_eq!(
        daily_proof_fee_adjust(&legacy.daily_fee, &raw_power, &full_qa_power),
        upgraded.daily_fee
    );
    assert_eq!(
        max(legacy.initial_pledge.clone(), current_initial_pledge(v, &full_qa_power)),
        upgraded.initial_pledge
    );

    // The power actor was told about the QA lift and the pledge top-up; no event is emitted.
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
                PowerPair { raw: BigInt::zero(), qa: &full_qa_power - &raw_power },
            ),
            Expect::power_update_pledge(miner_id, Some(pledge_delta)),
        ]),
        events: Some(vec![]),
        ..Default::default()
    }
    .matches(v.take_invocations().last().unwrap());
    assert_eq!(full_qa_power, miner_power(v, &maddr).qa);

    // Already at full power, the sector is not upgraded again: an in-place repeat changes
    // nothing, and one asking for an extension only extends, locking nothing and notifying
    // nobody.
    let new_expiration = v.epoch() + policy.max_sector_expiration_extension;
    for repeat in [None, Some(new_expiration)] {
        apply_ok(
            v,
            &worker,
            &maddr,
            &TokenAmount::zero(),
            MinerMethod::UpgradeSectorQuality as u64,
            Some(UpgradeSectorQualityParams {
                extensions: vec![UpgradeSectorQuality {
                    deadline,
                    partition,
                    sectors: sectors.clone(),
                    new_expiration: repeat,
                }],
            }),
        );
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
        let sector = sector_info(v, &maddr, sector_number);
        assert_eq!(repeat.unwrap_or(upgraded.expiration), sector.expiration);
        assert_eq!(upgraded.flags, sector.flags);
        assert_eq!(upgraded.initial_pledge, sector.initial_pledge);
        assert_eq!(upgraded.daily_fee, sector.daily_fee);
    }
    assert_eq!(v.epoch(), sector_info(v, &maddr, sector_number).power_base_epoch);

    // The sector keeps proving at its upgraded power.
    let (deadline_info, partition_index) = advance_to_proving_deadline(v, &maddr, sector_number);
    submit_windowed_post(v, &worker, &maddr, deadline_info, partition_index, None);
    advance_by_deadline_to_index(
        v,
        &maddr,
        (deadline_info.index + 1) % policy.wpost_period_deadlines,
    );
    assert_eq!(full_qa_power, miner_power(v, &maddr).qa);

    assert_invariants(v, &policy, None);
}
