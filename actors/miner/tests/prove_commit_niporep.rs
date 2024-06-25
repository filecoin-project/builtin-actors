use fil_actors_runtime::runtime::RuntimePolicy;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::{bigint::BigInt, clock::ChainEpoch, error::ExitCode};

use fil_actor_miner::{
    Actor, Method, SectorNIActivationInfo, SectorOnChainInfo, SectorOnChainInfoFlags,
    NI_AGGREGATE_FEE_BASE_SECTOR_COUNT,
};
use num_traits::Zero;
use util::*;

mod util;

const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn prove_zero_sectors_ni_fail() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();
    let policy = rt.policy();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let activation_epoch = seal_randomness_epoch + 400;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(activation_epoch);
    let params = h.make_prove_commit_ni_params(miner, &[], seal_randomness_epoch, expiration, 0);

    let res = rt.call::<Actor>(
        Method::ProveCommitSectorsNI as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    );

    assert!(res.is_err());
    assert_eq!(res.unwrap_err().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn prove_one_sector_aggregate_ni() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();
    let policy = rt.policy();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let activation_epoch = seal_randomness_epoch + 400;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(activation_epoch);

    let sector_nums = (0..1).collect::<Vec<_>>();
    let params =
        h.make_prove_commit_ni_params(miner, &sector_nums, seal_randomness_epoch, expiration, 0);

    let res = h.prove_commit_sectors_ni(&rt, params, true, noop());
    assert!(res.is_ok());

    let activation_results = res.unwrap().activation_results;
    assert_eq!(activation_results.success_count, 1);
    assert!(activation_results.all_ok());

    let deadlines = h.get_state(&rt).load_deadlines(&rt.store).unwrap();
    let deadline = deadlines.load_deadline(&rt.store, 0).unwrap();
    assert_eq!(deadline.live_sectors, 1);
}

#[test]
fn prove_sectors_ni_short_duration_fail() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();
    let policy = rt.policy();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let activation_epoch = seal_randomness_epoch + 400;
    let expiration = activation_epoch + policy.min_sector_expiration - 1;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(activation_epoch);

    let sector_nums = (0..1).collect::<Vec<_>>();
    let params =
        h.make_prove_commit_ni_params(miner, &sector_nums, seal_randomness_epoch, expiration, 0);

    let res = h.prove_commit_sectors_ni(&rt, params, true, noop());
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn prove_sectors_max_aggregate_ni() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();
    let policy = rt.policy();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let activation_epoch = seal_randomness_epoch + 400;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;
    let proving_deadline = 42;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(activation_epoch);

    let sector_nums = (0..rt.policy.max_aggregated_sectors_ni).collect::<Vec<_>>();
    let params = h.make_prove_commit_ni_params(
        miner,
        &sector_nums,
        seal_randomness_epoch,
        expiration,
        proving_deadline,
    );
    let seal_proof_type = params.seal_proof_type;

    let res = h.prove_commit_sectors_ni(&rt, params, true, noop());

    assert!(res.is_ok());

    let activation_results = res.unwrap().activation_results;
    assert_eq!(activation_results.success_count, rt.policy.max_aggregated_sectors_ni as u32);
    assert!(activation_results.all_ok());

    let deadlines = h.get_state(&rt).load_deadlines(&rt.store).unwrap();
    let deadline = deadlines.load_deadline(&rt.store, proving_deadline).unwrap();
    let partitions = deadline.partitions_amt(&rt.store).unwrap();
    let partition = partitions.get(0).unwrap().unwrap();
    let partition_sectors: Vec<u64> = partition.sectors.iter().collect();

    assert_eq!(deadline.live_sectors, rt.policy.max_aggregated_sectors_ni);

    // Check if the sectors in partition are the ones we just committed
    assert!(partition_sectors
        .iter()
        .rev()
        .take(rt.policy.max_aggregated_sectors_ni as usize)
        .rev()
        .eq(sector_nums.iter()));

    let sectors: Vec<SectorOnChainInfo> =
        sector_nums.iter().map(|sector_num| h.get_sector(&rt, *sector_num)).collect();

    for (on_chain_sector, sector_num) in sectors.iter().zip(sector_nums) {
        assert_eq!(sector_num, on_chain_sector.sector_number);
        assert_eq!(seal_proof_type, on_chain_sector.seal_proof);
        assert!(on_chain_sector.deprecated_deal_ids.is_empty());
        assert_eq!(activation_epoch, on_chain_sector.activation);
        assert_eq!(expiration, on_chain_sector.expiration);
        assert_eq!(BigInt::zero(), on_chain_sector.deal_weight);
        assert_eq!(BigInt::zero(), on_chain_sector.verified_deal_weight);
        assert_eq!(activation_epoch, on_chain_sector.power_base_epoch);
        assert!(on_chain_sector.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER));
    }
}

#[test]
fn ni_prove_partialy_valid_sectors_not_required_activation() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();
    let policy = rt.policy();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let activation_epoch = seal_randomness_epoch + 400;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;
    let proving_deadline = 42;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(activation_epoch);

    let num_success: usize = 2;
    let sector_nums =
        (0..((NI_AGGREGATE_FEE_BASE_SECTOR_COUNT + num_success) as u64)).collect::<Vec<_>>();
    let num_fails = NI_AGGREGATE_FEE_BASE_SECTOR_COUNT;
    let mut params = h.make_prove_commit_ni_params(
        miner,
        &sector_nums,
        seal_randomness_epoch,
        expiration,
        proving_deadline,
    );
    params.require_activation_success = false;

    let seal_proof_type = params.seal_proof_type;

    // Purposefully fail some sectors by setting the seal_rand_epoch to the activation_epoch
    let res = h.prove_commit_sectors_ni(
        &rt,
        params,
        true,
        fail_for_seal_rand_epoch(
            num_fails,
            activation_epoch - policy.max_prove_commit_ni_randomness_lookback - 1,
        ),
    );
    assert!(res.is_ok());

    let activation_results = res.unwrap().activation_results;
    assert_eq!(activation_results.success_count, num_success as u32);
    assert_eq!(activation_results.fail_codes.len(), num_fails);

    let deadlines = h.get_state(&rt).load_deadlines(&rt.store).unwrap();
    let deadline = deadlines.load_deadline(&rt.store, proving_deadline).unwrap();
    let partitions = deadline.partitions_amt(&rt.store).unwrap();
    let partition = partitions.get(0).unwrap().unwrap();
    let partition_sectors: Vec<u64> = partition.sectors.iter().collect();

    assert_eq!(deadline.live_sectors, num_success as u64);

    // Check if the sectors in partition are the ones we just committed
    let success_sectors = sector_nums.into_iter().skip(num_fails).collect::<Vec<_>>();
    assert!(partition_sectors.iter().rev().take(num_success).rev().eq(success_sectors.iter()));

    let sectors: Vec<SectorOnChainInfo> =
        success_sectors.iter().map(|sector_num| h.get_sector(&rt, *sector_num)).collect();

    for (on_chain_sector, sector_num) in sectors.iter().zip(success_sectors) {
        assert_eq!(sector_num, on_chain_sector.sector_number);
        assert_eq!(seal_proof_type, on_chain_sector.seal_proof);
        assert!(on_chain_sector.deprecated_deal_ids.is_empty());
        assert_eq!(activation_epoch, on_chain_sector.activation);
        assert_eq!(expiration, on_chain_sector.expiration);
        assert_eq!(BigInt::zero(), on_chain_sector.deal_weight);
        assert_eq!(BigInt::zero(), on_chain_sector.verified_deal_weight);
        assert_eq!(activation_epoch, on_chain_sector.power_base_epoch);
        assert!(on_chain_sector.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER));
    }
}

#[test]
fn ni_prove_partialy_valid_sectors_by_sealer_id_not_required_activation() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();
    let policy = rt.policy();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let activation_epoch = seal_randomness_epoch + 400;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;
    let proving_deadline = 42;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(activation_epoch);

    let sector_nums = (0..rt.policy.max_aggregated_sectors_ni).collect::<Vec<_>>();
    let num_fails = sector_nums.len() / 2;
    let num_success = sector_nums.len() - num_fails;
    let mut params = h.make_prove_commit_ni_params(
        miner,
        &sector_nums,
        seal_randomness_epoch,
        expiration,
        proving_deadline,
    );
    params.require_activation_success = false;

    let seal_proof_type = params.seal_proof_type;

    // Purposefully fail some sectors by setting invalid sealer id
    let res =
        h.prove_commit_sectors_ni(&rt, params, true, fail_for_invalid_sealer_id(num_fails, miner));
    assert!(res.is_ok());

    let activation_results = res.unwrap().activation_results;
    assert_eq!(activation_results.success_count, num_success as u32);
    assert_eq!(activation_results.fail_codes.len(), num_fails);

    let deadlines = h.get_state(&rt).load_deadlines(&rt.store).unwrap();
    let deadline = deadlines.load_deadline(&rt.store, proving_deadline).unwrap();
    let partitions = deadline.partitions_amt(&rt.store).unwrap();
    let partition = partitions.get(0).unwrap().unwrap();
    let partition_sectors: Vec<u64> = partition.sectors.iter().collect();

    assert_eq!(deadline.live_sectors, num_success as u64);

    // Check if the sectors in partition are the ones we just committed
    let success_sectors = sector_nums.into_iter().skip(num_fails).collect::<Vec<_>>();
    assert!(partition_sectors.iter().rev().take(num_success).rev().eq(success_sectors.iter()));

    let sectors: Vec<SectorOnChainInfo> =
        success_sectors.iter().map(|sector_num| h.get_sector(&rt, *sector_num)).collect();

    for (on_chain_sector, sector_num) in sectors.iter().zip(success_sectors) {
        assert_eq!(sector_num, on_chain_sector.sector_number);
        assert_eq!(seal_proof_type, on_chain_sector.seal_proof);
        assert!(on_chain_sector.deprecated_deal_ids.is_empty());
        assert_eq!(activation_epoch, on_chain_sector.activation);
        assert_eq!(expiration, on_chain_sector.expiration);
        assert_eq!(BigInt::zero(), on_chain_sector.deal_weight);
        assert_eq!(BigInt::zero(), on_chain_sector.verified_deal_weight);
        assert_eq!(activation_epoch, on_chain_sector.power_base_epoch);
        assert!(on_chain_sector.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER));
    }
}

#[test]
fn ni_prove_partialy_valid_sectors_required_activation() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();
    let policy = rt.policy();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let activation_epoch = seal_randomness_epoch + 400;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;
    let proving_deadline = 42;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(activation_epoch);

    let sector_nums = (0..rt.policy.max_aggregated_sectors_ni).collect::<Vec<_>>();
    let mut params = h.make_prove_commit_ni_params(
        miner,
        &sector_nums,
        seal_randomness_epoch,
        expiration,
        proving_deadline,
    );
    params.require_activation_success = true;

    // Purposefully fail some sectors by setting the seal_rand_epoch to the activation_epoch
    params.sectors[0].seal_rand_epoch = activation_epoch;

    let res =
        h.prove_commit_sectors_ni(&rt, params, true, fail_for_seal_rand_epoch(1, activation_epoch));
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn prove_sectors_multiple_max_aggregate_ni() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();
    let policy = rt.policy();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let activation_epoch = seal_randomness_epoch + 400;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;
    let proving_deadline = 42;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(activation_epoch);

    // Iterating multiple times to verify that the all sectors will be in the same deadline
    for i in 0..3 {
        let sector_nums =
            (i * 1000..i * 1000 + rt.policy.max_aggregated_sectors_ni).collect::<Vec<_>>();
        let params = h.make_prove_commit_ni_params(
            miner,
            &sector_nums,
            seal_randomness_epoch,
            expiration,
            proving_deadline,
        );
        let seal_proof_type = params.seal_proof_type;

        let res = h.prove_commit_sectors_ni(&rt, params, i == 0, noop());
        assert!(res.is_ok());

        let deadlines = h.get_state(&rt).load_deadlines(&rt.store).unwrap();
        let deadline = deadlines.load_deadline(&rt.store, proving_deadline).unwrap();
        let partitions = deadline.partitions_amt(&rt.store).unwrap();
        let partition = partitions.get(0).unwrap().unwrap();
        let partition_sectors: Vec<u64> = partition.sectors.iter().collect();

        assert_eq!(deadline.live_sectors, (i + 1) * rt.policy.max_aggregated_sectors_ni);

        // Check if the last max_aggregated_sectors_ni sectors in partition are the ones we just committed
        assert!(partition_sectors
            .iter()
            .rev()
            .take(rt.policy.max_aggregated_sectors_ni as usize)
            .rev()
            .eq(sector_nums.iter()));

        let sectors: Vec<SectorOnChainInfo> =
            sector_nums.iter().map(|sector_num| h.get_sector(&rt, *sector_num)).collect();

        for (on_chain_sector, sector_num) in sectors.iter().zip(sector_nums) {
            assert_eq!(sector_num, on_chain_sector.sector_number);
            assert_eq!(seal_proof_type, on_chain_sector.seal_proof);
            assert!(on_chain_sector.deprecated_deal_ids.is_empty());
            assert_eq!(activation_epoch, on_chain_sector.activation);
            assert_eq!(expiration, on_chain_sector.expiration);
            assert_eq!(BigInt::zero(), on_chain_sector.deal_weight);
            assert_eq!(BigInt::zero(), on_chain_sector.verified_deal_weight);
            assert_eq!(activation_epoch, on_chain_sector.power_base_epoch);
            assert!(on_chain_sector.flags.contains(SectorOnChainInfoFlags::SIMPLE_QA_POWER));
        }
    }
}

#[test]
fn prove_too_many_sectors_ni_fail() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();
    let policy = rt.policy();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let activation_epoch = seal_randomness_epoch + 400;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(activation_epoch);

    let sector_nums = (0..rt.policy.max_aggregated_sectors_ni + 1).collect::<Vec<_>>();

    let params =
        h.make_prove_commit_ni_params(miner, &sector_nums, seal_randomness_epoch, expiration, 0);

    let res = rt.call::<Actor>(
        Method::ProveCommitSectorsNI as u64,
        IpldBlock::serialize_cbor(&params).unwrap(),
    );

    assert!(res.is_err());
    assert_eq!(res.unwrap_err().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn ni_prove_fail_sector_number_already_in_use() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();
    let policy = rt.policy();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let activation_epoch = seal_randomness_epoch + 400;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;
    let proving_deadline = 42;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(activation_epoch);

    let sector_nums = (0..10).collect::<Vec<_>>();
    let params = h.make_prove_commit_ni_params(
        miner,
        &sector_nums,
        seal_randomness_epoch,
        expiration,
        proving_deadline,
    );

    let res = h.prove_commit_sectors_ni(&rt, params, true, noop());
    assert!(res.is_ok());

    // Overlap sectors nums with the previous sectors nums that were committed
    let sector_nums = (9..20).collect::<Vec<_>>();
    let params = h.make_prove_commit_ni_params(
        miner,
        &sector_nums,
        seal_randomness_epoch,
        expiration,
        proving_deadline,
    );
    let res = h.prove_commit_sectors_ni(&rt, params, false, noop());
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

#[test]
fn ni_prove_fail_duplicated_sector_numbers() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());
    let miner = rt.receiver.id().unwrap();
    let policy = rt.policy();

    let seal_randomness_epoch = PERIOD_OFFSET + 1;
    let activation_epoch = seal_randomness_epoch + 400;
    let expiration = activation_epoch + policy.min_sector_expiration + 1;
    let proving_deadline = 42;

    rt.set_epoch(seal_randomness_epoch);
    h.construct_and_verify(&rt);

    rt.set_epoch(activation_epoch);

    let mut sector_nums = (0..10).collect::<Vec<_>>();
    // Duplicate the first sector number
    sector_nums.push(*sector_nums.first().unwrap());

    let params = h.make_prove_commit_ni_params(
        miner,
        &sector_nums,
        seal_randomness_epoch,
        expiration,
        proving_deadline,
    );

    let res = h.prove_commit_sectors_ni(&rt, params, true, noop());
    assert!(res.is_err());
    assert_eq!(res.unwrap_err().exit_code(), ExitCode::USR_ILLEGAL_ARGUMENT);
}

fn fail_for_seal_rand_epoch(
    num_fails: usize,
    bad_seal_rand_epoch: i64,
) -> impl FnMut(&mut SectorNIActivationInfo, usize) -> bool {
    move |s: &mut SectorNIActivationInfo, index: usize| {
        if index < num_fails {
            s.seal_rand_epoch = bad_seal_rand_epoch;
            true
        } else {
            false
        }
    }
}

fn fail_for_invalid_sealer_id(
    num_fails: usize,
    miner_id: u64,
) -> impl FnMut(&mut SectorNIActivationInfo, usize) -> bool {
    move |s: &mut SectorNIActivationInfo, index: usize| {
        if index < num_fails {
            s.sealer_id = miner_id + 1;
            true
        } else {
            false
        }
    }
}

fn noop() -> impl FnMut(&mut SectorNIActivationInfo, usize) -> bool {
    |_, _| false
}
