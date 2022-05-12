use fil_actor_market::SectorWeights;
use fil_actor_miner::PreCommitSectorBatchParams;
use fil_actor_miner::{
    initial_pledge_for_power, pre_commit_deposit_for_power, qa_power_for_weight, PowerPair,
};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::DealWeight;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
use fvm_shared::sector::MAX_SECTOR_NUMBER;
use std::collections::HashMap;

mod util;
use util::*;

// an expriration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

const VERIFIED_DEAL_WEIGHT_MULTIPLIER: u64 = 100;
const QUALITY_BASE_MULTIPLIER: u64 = 10;
const BIG_BALANCE: u128 = 1_000_000_000_000_000_000_000_000u128;
const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn prove_single_sector() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.balance.replace(TokenAmount::from(BIG_BALANCE));

    let precommit_epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);

    h.construct_and_verify(&mut rt);
    let dl_info = h.deadline(&rt);

    // Make a good commitment for the proof to target.
    // Use the max sector number to make sure everything works.
    let sector_no = MAX_SECTOR_NUMBER;
    let prove_commit_epoch = precommit_epoch + rt.policy.pre_commit_challenge_delay + 1;
    let expiration =
        dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period; // something on deadline boundary but > 180 days
                                                                                           // Fill the sector with verified deals
    let sector_weight =
        DealWeight::from(h.sector_size as u64) * DealWeight::from(expiration - prove_commit_epoch);
    let deal_weight = DealWeight::zero();
    let verified_deal_weight = sector_weight;

    // Pre-commit with a deal in order to exercise non-zero deal weights.
    let precommit_params =
        h.make_pre_commit_params(sector_no, precommit_epoch - 1, expiration, vec![1]);
    let precommit = h.pre_commit_sector(
        &mut rt,
        precommit_params,
        PreCommitConfig {
            deal_weight: deal_weight.clone(),
            verified_deal_weight: verified_deal_weight.clone(),
            deal_space: None,
        },
        true,
    );

    // Check precommit
    // deal weights must be set in precommit onchain info
    assert_eq!(&deal_weight, &precommit.deal_weight);
    assert_eq!(&verified_deal_weight, &precommit.verified_deal_weight);

    let pwr_estimate = qa_power_for_weight(
        h.sector_size,
        precommit.info.expiration - precommit_epoch,
        &precommit.deal_weight,
        &precommit.verified_deal_weight,
    );
    let expected_deposit = pre_commit_deposit_for_power(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &pwr_estimate,
    );
    assert_eq!(expected_deposit, precommit.pre_commit_deposit);

    // expect total precommit deposit to equal our new deposit
    let st = h.get_state(&rt);
    assert_eq!(expected_deposit, st.pre_commit_deposits);

    // run prove commit logic
    rt.set_epoch(prove_commit_epoch);
    rt.balance.replace(TokenAmount::from(1000) * 1e18 as u64);
    let sector = h.prove_commit_sector_and_confirm(
        &mut rt,
        &precommit,
        h.make_prove_commit_params(sector_no),
        ProveCommitConfig::default(),
    );

    assert_eq!(precommit.info.seal_proof, sector.seal_proof);
    assert_eq!(precommit.info.sealed_cid, sector.sealed_cid);
    assert_eq!(precommit.info.deal_ids, sector.deal_ids);
    assert_eq!(rt.epoch, sector.activation);
    assert_eq!(precommit.info.expiration, sector.expiration);
    assert_eq!(precommit.deal_weight, sector.deal_weight);
    assert_eq!(precommit.verified_deal_weight, sector.verified_deal_weight);

    // expect precommit to have been removed
    let st = h.get_state(&rt);
    let found = st.get_precommitted_sector(&rt.store, sector_no).unwrap();
    assert!(found.is_none());

    // expect deposit to have been transferred to initial pledges
    assert!(st.pre_commit_deposits.is_zero());

    // The sector is exactly full with verified deals, so expect fully verified power.
    let expected_power = StoragePower::from(h.sector_size as u64)
        * (VERIFIED_DEAL_WEIGHT_MULTIPLIER / QUALITY_BASE_MULTIPLIER);
    let qa_power = qa_power_for_weight(
        h.sector_size,
        precommit.info.expiration - rt.epoch,
        &precommit.deal_weight,
        &precommit.verified_deal_weight,
    );
    assert_eq!(expected_power, qa_power);
    let sector_power =
        PowerPair { raw: StoragePower::from(h.sector_size as u64), qa: qa_power.clone() };

    // expect deal weights to be transferred to on chain info
    assert_eq!(precommit.deal_weight, sector.deal_weight);
    assert_eq!(precommit.verified_deal_weight, sector.verified_deal_weight);

    // expect initial plege of sector to be set, and be total pledge requirement
    let expected_initial_pledge = initial_pledge_for_power(
        &qa_power,
        &h.baseline_power,
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &rt.total_fil_circ_supply(),
    );
    assert_eq!(expected_initial_pledge, sector.initial_pledge);
    assert_eq!(expected_initial_pledge, st.initial_pledge);

    // expect sector to be assigned a deadline/partition
    let (dl_idx, p_idx) = st.find_sector(&rt.policy, rt.store(), sector_no).unwrap();
    let (deadline, partition) = h.get_deadline_and_partition(&rt, dl_idx, p_idx);
    assert_eq!(1, deadline.live_sectors);
    assert!(deadline.partitions_posted.is_empty());
    assert!(deadline.early_terminations.is_empty());

    let quant = st.quant_spec_for_deadline(&rt.policy, dl_idx);
    let quantized_expiration = quant.quantize_up(precommit.info.expiration);

    let d_queue = h.collect_deadline_expirations(&rt, &deadline);
    assert_eq!(HashMap::from([(quantized_expiration, vec![p_idx])]), d_queue);

    assert_bitfield_equals(&partition.sectors, &[sector_no]);
    assert!(partition.faults.is_empty());
    assert!(partition.recoveries.is_empty());
    assert!(partition.terminated.is_empty());
    assert_eq!(sector_power, partition.live_power);
    assert!(partition.faulty_power.is_zero());
    assert!(partition.recovering_power.is_zero());

    let p_queue = h.collect_partition_expirations(&rt, &partition);
    let entry = &p_queue[&quantized_expiration];
    assert_bitfield_equals(&entry.on_time_sectors, &[sector_no]);
    assert!(entry.early_sectors.is_empty());
    assert_eq!(expected_initial_pledge, entry.on_time_pledge);
    assert_eq!(sector_power, entry.active_power);
    assert!(entry.faulty_power.is_zero());
}

#[test]
fn prove_sectors_from_batch_pre_commit() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.balance.replace(TokenAmount::from(BIG_BALANCE));

    let precommit_epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);

    h.construct_and_verify(&mut rt);
    let dl_info = h.deadline(&rt);

    let sector_expiration =
        dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period;

    let sectors = vec![
        h.make_pre_commit_params(100, precommit_epoch - 1, sector_expiration, vec![]),
        h.make_pre_commit_params(101, precommit_epoch - 1, sector_expiration, vec![1]), // 1 * 32GiB verified deal
        h.make_pre_commit_params(102, precommit_epoch - 1, sector_expiration, vec![2, 3]), // 2 * 16GiB verified deals
    ];

    let deal_space = 32 << 30;
    let deal_weight = DealWeight::zero();
    let prove_commit_epoch = precommit_epoch + rt.policy.pre_commit_challenge_delay + 1;
    let deal_lifespan = sector_expiration - prove_commit_epoch;
    let verified_deal_weight = deal_space * DealWeight::from(deal_lifespan);

    // Power estimates made a pre-commit time
    let no_deal_power_estimate = qa_power_for_weight(
        h.sector_size,
        sector_expiration - precommit_epoch,
        &DealWeight::zero(),
        &DealWeight::zero(),
    );
    let full_deal_power_estimate = qa_power_for_weight(
        h.sector_size,
        sector_expiration - precommit_epoch,
        &deal_weight,
        &verified_deal_weight,
    );

    let deposits = [
        pre_commit_deposit_for_power(
            &h.epoch_reward_smooth,
            &h.epoch_qa_power_smooth,
            &no_deal_power_estimate,
        ),
        pre_commit_deposit_for_power(
            &h.epoch_reward_smooth,
            &h.epoch_qa_power_smooth,
            &full_deal_power_estimate,
        ),
        pre_commit_deposit_for_power(
            &h.epoch_reward_smooth,
            &h.epoch_qa_power_smooth,
            &full_deal_power_estimate,
        ),
    ];
    let conf = PreCommitBatchConfig {
        sector_weights: vec![
            SectorWeights {
                deal_space: 0,
                deal_weight: DealWeight::zero(),
                verified_deal_weight: DealWeight::zero(),
            },
            SectorWeights {
                deal_space,
                deal_weight: deal_weight.clone(),
                verified_deal_weight: verified_deal_weight.clone(),
            },
            SectorWeights {
                deal_space,
                deal_weight: deal_weight.clone(),
                verified_deal_weight: verified_deal_weight.clone(),
            },
        ],
        first_for_miner: true,
    };

    let precommits = h.pre_commit_sector_batch(
        &mut rt,
        PreCommitSectorBatchParams { sectors },
        conf,
        TokenAmount::zero(),
    );

    rt.set_epoch(prove_commit_epoch);

    let no_deal_power = qa_power_for_weight(
        h.sector_size,
        sector_expiration - prove_commit_epoch,
        &DealWeight::zero(),
        &DealWeight::zero(),
    );
    let no_deal_pledge = initial_pledge_for_power(
        &no_deal_power,
        &h.baseline_power,
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &rt.total_fil_circ_supply(),
    );
    let full_deal_power = qa_power_for_weight(
        h.sector_size,
        sector_expiration - prove_commit_epoch,
        &deal_weight,
        &verified_deal_weight,
    );
    let expected_power = StoragePower::from(h.sector_size as u64)
        * (VERIFIED_DEAL_WEIGHT_MULTIPLIER / QUALITY_BASE_MULTIPLIER);
    assert_eq!(full_deal_power, expected_power);
    let full_deal_pledge = initial_pledge_for_power(
        &full_deal_power,
        &h.baseline_power,
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &rt.total_fil_circ_supply(),
    );

    // Prove just the first sector, with no deals
    {
        let precommit = &precommits[0];
        let sector = h.prove_commit_sector_and_confirm(
            &mut rt,
            precommit,
            h.make_prove_commit_params(precommit.info.sector_number),
            ProveCommitConfig::default(),
        );
        assert_eq!(rt.epoch, sector.activation);
        let st = h.get_state(&rt);
        let expected_deposit: TokenAmount = deposits[1..].iter().sum(); // First sector deposit released
        assert_eq!(expected_deposit, st.pre_commit_deposits);

        // Expect power/pledge for a sector with no deals
        assert_eq!(no_deal_pledge, sector.initial_pledge);
        assert_eq!(no_deal_pledge, st.initial_pledge);
    }
    // Prove the next, with one deal
    {
        let precommit = &precommits[1];
        let sector = h.prove_commit_sector_and_confirm(
            &mut rt,
            precommit,
            h.make_prove_commit_params(precommit.info.sector_number),
            ProveCommitConfig::default(),
        );
        assert_eq!(rt.epoch, sector.activation);
        let st = h.get_state(&rt);
        let expected_deposit: TokenAmount = deposits[2..].iter().sum(); // First and second sector deposits released
        assert_eq!(expected_deposit, st.pre_commit_deposits);

        // Expect power/pledge for the two sectors (only this one having any deal weight)
        assert_eq!(full_deal_pledge, sector.initial_pledge);
        assert_eq!(&no_deal_pledge + &full_deal_pledge, st.initial_pledge);
    }
    // Prove the last
    {
        let precommit = &precommits[2];
        let sector = h.prove_commit_sector_and_confirm(
            &mut rt,
            precommit,
            h.make_prove_commit_params(precommit.info.sector_number),
            ProveCommitConfig::default(),
        );
        assert_eq!(rt.epoch, sector.activation);
        let st = h.get_state(&rt);
        assert!(st.pre_commit_deposits.is_zero());

        // Expect power/pledge for the three sectors
        assert_eq!(&full_deal_pledge, &sector.initial_pledge);
        assert_eq!(&no_deal_pledge + &full_deal_pledge + &full_deal_pledge, st.initial_pledge);
    }
}
