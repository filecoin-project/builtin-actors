// Ad-hoc benchmark (not part of the regular suite) for measuring blockstore read/write
// activity when a storage provider terminates a batch of sectors, most of them carrying
// verified (DataCap-backed) deals -- the scenario people report as unexpectedly expensive.
//
// Compare before/after the TerminateSectors + OnMinerSectorsTerminate batching fix by
// running this same file against both revisions:
//   cargo test -p test_vm --test terminate_gas_bench -- --nocapture

use fil_actor_market::Method as MarketMethod;
use fil_actor_miner::{Method as MinerMethod, TerminateSectorsParams, TerminationDeclaration};
use fil_actors_integration_tests::util::{
    advance_by_deadline_to_epoch, advance_to_proving_deadline, create_accounts, create_miner,
    cron_tick, make_bitfield, make_piece_manifests_from_deal_ids, market_publish_deal,
    miner_prove_sector, precommit_meta_data_from_deals, precommit_sectors_v2, sector_deadline,
    submit_windowed_post, verifreg_add_verifier,
};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_blockstores::{BSStats, MemoryBlockstore};
use fil_actors_runtime::{STORAGE_MARKET_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR};
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::{RegisteredSealProof, StoragePower};
use num_traits::Zero;
use num_traits::cast::FromPrimitive;
use test_vm::TestVM;
use vm_api::VM;
use vm_api::util::apply_ok;

/// Number of sectors, each with its own verified deal, that the miner will terminate in one
/// TerminateSectors call. All are sized to fit in a single partition (32GiB partitions hold up
/// to 2349 sectors) so the scenario stresses deal count without needing to seal thousands of
/// sectors just to spill into a second partition.
const NUM_SECTORS: u64 = 1000;

#[test]
fn terminate_sectors_gas_bench() {
    let store = MemoryBlockstore::new();
    let v = TestVM::new_with_singletons(store);

    let addrs = create_accounts(&v, 2, &TokenAmount::from_whole(100_000));
    let (owner, verified_client) = (addrs[0], addrs[1]);
    let worker = owner;

    let seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
    let (miner_id_addr, miner_robust_addr) = create_miner(
        &v,
        &owner,
        &worker,
        seal_proof.registered_window_post_proof().unwrap(),
        &TokenAmount::from_whole(20 * NUM_SECTORS as i64 + 1_000),
    );

    verifreg_add_verifier(&v, &owner, StoragePower::from_i64(32i64 << 40).unwrap());
    apply_ok(
        &v,
        &owner,
        &VERIFIED_REGISTRY_ACTOR_ADDR,
        &TokenAmount::zero(),
        fil_actor_verifreg::Method::AddVerifiedClient as u64,
        Some(fil_actor_verifreg::VerifierParams {
            address: verified_client,
            allowance: StoragePower::from_i64(32i64 << 40).unwrap(),
        }),
    );

    apply_ok(
        &v,
        &verified_client,
        &STORAGE_MARKET_ACTOR_ADDR,
        &TokenAmount::from_whole(3 * NUM_SECTORS as i64),
        MarketMethod::AddBalance as u64,
        Some(verified_client),
    );
    apply_ok(
        &v,
        &worker,
        &STORAGE_MARKET_ACTOR_ADDR,
        // 2 FIL provider collateral per deal, plus margin.
        &TokenAmount::from_whole(3 * NUM_SECTORS as i64),
        MarketMethod::AddBalance as u64,
        Some(miner_id_addr),
    );

    let sector_number_base = 100u64;
    const DEAL_LIFETIME: i64 = 400 * 2880; // ~400 days
    // A verified-deal claim requires term_min(=deal lifetime) <= sector_lifetime <=
    // term_max(=deal lifetime + 90-day buffer, capped). Keep comfortably inside that window.
    const SECTOR_LIFETIME: i64 = 450 * 2880; // ~450 days
    let deal_start = v.epoch() + Policy::default().pre_commit_challenge_delay + 1;
    let expiration = deal_start + SECTOR_LIFETIME;

    // Publish one verified deal per sector, then precommit all sectors together in a single
    // batch message: sectors precommitted together are packed into the same deadline/partition
    // (up to partition capacity -- 2349 sectors for a 32GiB partition), rather than spread across
    // deadlines the way one-sector-at-a-time precommits (each its own message, on its own
    // schedule) would be. That packing is what lets a real SP's later TerminateSectors call name
    // just one (deadline, partition) pair covering many sectors -- and many deals.
    let mut deal_ids_by_sector = vec![];
    for i in 0..NUM_SECTORS {
        let deals = market_publish_deal(
            &v,
            &worker,
            &verified_client,
            &miner_id_addr,
            format!("deal{i}"),
            PaddedPieceSize(1 << 30),
            true,
            deal_start,
            DEAL_LIFETIME,
        );
        deal_ids_by_sector.push(deals.ids.to_vec());
    }
    cron_tick(&v);

    let metadata = deal_ids_by_sector
        .iter()
        .map(|deal_ids| precommit_meta_data_from_deals(&v, deal_ids, seal_proof, false))
        .collect();
    precommit_sectors_v2(
        &v,
        NUM_SECTORS as usize,
        metadata,
        &worker,
        &miner_robust_addr,
        seal_proof,
        sector_number_base,
        true,
        Some(expiration),
    );

    let prove_time = v.epoch() + Policy::default().pre_commit_challenge_delay + 1;
    advance_by_deadline_to_epoch(&v, &miner_id_addr, prove_time);

    let mut sector_numbers = vec![];
    for (i, deal_ids) in deal_ids_by_sector.into_iter().enumerate() {
        let sector_number = sector_number_base + i as u64;
        miner_prove_sector(
            &v,
            &worker,
            &miner_id_addr,
            sector_number,
            make_piece_manifests_from_deal_ids(&v, deal_ids),
        );
        sector_numbers.push(sector_number);
    }

    // All sectors should have packed into the same deadline/partition.
    let (d_idx, p_idx) = sector_deadline(&v, &miner_id_addr, sector_numbers[0]);
    for &s in &sector_numbers {
        assert_eq!((d_idx, p_idx), sector_deadline(&v, &miner_id_addr, s));
    }

    let (dline_info, p_idx) = advance_to_proving_deadline(&v, &miner_id_addr, sector_numbers[0]);
    submit_windowed_post(&v, &worker, &miner_id_addr, dline_info, p_idx, None);
    v.set_epoch(dline_info.close);
    advance_by_deadline_to_epoch(
        &v,
        &miner_id_addr,
        dline_info.close + Policy::default().deal_updates_interval,
    );

    let before: BSStats = *v.store.stats.borrow();

    apply_ok(
        &v,
        &worker,
        &miner_robust_addr,
        &TokenAmount::zero(),
        MinerMethod::TerminateSectors as u64,
        Some(TerminateSectorsParams {
            terminations: vec![TerminationDeclaration {
                deadline: d_idx,
                partition: p_idx,
                sectors: make_bitfield(&sector_numbers),
            }],
        }),
    );

    let after: BSStats = *v.store.stats.borrow();

    println!(
        "TerminateSectors ({NUM_SECTORS} sectors, {NUM_SECTORS} verified deals): \
         reads +{} writes +{} bytes_read +{} bytes_written +{}",
        after.r - before.r,
        after.w - before.w,
        after.br - before.br,
        after.bw - before.bw,
    );
}
