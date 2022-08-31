use fil_actor_market::SectorWeights;
use fil_actor_miner::{
    aggregate_pre_commit_network_fee, max_prove_commit_duration, pre_commit_deposit_for_power,
    qa_power_for_weight, PreCommitSectorBatchParams, SectorPreCommitInfo, State,
};
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::*;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::SectorNumber;

use num_traits::Zero;

use std::collections::HashMap;

mod util;
use util::*;

// an expiration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: i64 = 220;

// A balance for use in tests where the miner's low balance is not interesting.

#[derive(Default, Clone)]
struct DealSpec {
    size: u64,
    verified_size: u64,
    ids: Vec<DealID>,
}

fn assert_simple_batch(
    batch_size: usize,
    balance_surplus: TokenAmount,
    base_fee: TokenAmount,
    deal_specs: &[DealSpec],
    exit_code: ExitCode,
    error_str: &str,
) {
    let period_offset = ChainEpoch::from(100);

    let h = ActorHarness::new(period_offset);
    let mut rt = h.new_runtime();

    let precommit_epoch = period_offset + 1;
    rt.set_epoch(precommit_epoch);
    h.construct_and_verify(&mut rt);
    let dl_info = h.deadline(&rt);

    let sector_nos: Vec<SectorNumber> = (0..batch_size).map(|x| x as u64 + 100).collect();
    let sector_no_as_uints: Vec<u64> = (0..batch_size).map(|x| x as u64 + 100).collect();
    let sector_expiration =
        dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period; // on deadline boundary but > 180 days
    let prove_commit_epoch = precommit_epoch + rt.policy.pre_commit_challenge_delay + 1;
    let deal_lifespan = sector_expiration - prove_commit_epoch;

    let mut sectors = vec![SectorPreCommitInfo::default(); batch_size];
    let mut conf = PreCommitBatchConfig {
        sector_weights: vec![SectorWeights::default(); batch_size],
        first_for_miner: true,
    };
    let mut deposits = vec![TokenAmount::zero(); batch_size];

    for i in 0..batch_size {
        let mut deals = DealSpec::default();
        if deal_specs.len() > i {
            deals = deal_specs[i].clone();
        }
        sectors[i] = h.make_pre_commit_params(
            sector_nos[i],
            precommit_epoch - 1,
            sector_expiration,
            deals.ids,
        );

        let deal_space = deals.size + deals.verified_size;
        let deal_weight = BigInt::from(deals.size) * BigInt::from(deal_lifespan);
        let verified_deal_weight = BigInt::from(deals.verified_size) * BigInt::from(deal_lifespan);
        conf.sector_weights[i] = SectorWeights {
            deal_space,
            deal_weight: deal_weight.clone(),
            verified_deal_weight: verified_deal_weight.clone(),
        };
        let pwr_estimate = qa_power_for_weight(
            h.sector_size,
            sectors[i].expiration - precommit_epoch,
            &deal_weight,
            &verified_deal_weight,
        );
        deposits[i] = pre_commit_deposit_for_power(
            &h.epoch_reward_smooth,
            &h.epoch_qa_power_smooth,
            &pwr_estimate,
        );
    }
    let net_fee = aggregate_pre_commit_network_fee(batch_size as i64, &base_fee);
    let total_deposit: TokenAmount = deposits.iter().sum();
    let total_balance = net_fee + &total_deposit;
    rt.set_balance(total_balance + balance_surplus);

    if exit_code != ExitCode::OK {
        expect_abort_contains_message(
            exit_code,
            error_str,
            h.pre_commit_sector_batch(
                &mut rt,
                PreCommitSectorBatchParams { sectors },
                &conf,
                &base_fee,
            ),
        );
        rt.reset();

        // State untouched.
        let st: State = rt.get_state();
        assert!(st.pre_commit_deposits.is_zero());
        let expirations = h.collect_precommit_expirations(&rt, &st);
        assert!(expirations.is_empty());
        return;
    }
    let precommits = h.pre_commit_sector_batch_and_get(
        &mut rt,
        PreCommitSectorBatchParams { sectors: sectors.clone() },
        &conf,
        &base_fee,
    );

    // Check precommits
    let st: State = rt.get_state();
    for i in (0..batch_size).map(|i| i as usize) {
        assert_eq!(precommit_epoch, precommits[i].pre_commit_epoch);
        assert_eq!(conf.sector_weights[i].deal_weight, precommits[i].deal_weight);
        assert_eq!(conf.sector_weights[i].verified_deal_weight, precommits[i].verified_deal_weight);

        assert_eq!(sector_nos[i], precommits[i].info.sector_number);
        assert_eq!(sectors[i], precommits[i].info);

        let pwr_estimate = qa_power_for_weight(
            h.sector_size,
            sectors[i].expiration - precommit_epoch,
            &conf.sector_weights[i].deal_weight,
            &conf.sector_weights[i].verified_deal_weight,
        );
        let expect_deposit = pre_commit_deposit_for_power(
            &h.epoch_reward_smooth,
            &h.epoch_qa_power_smooth,
            &pwr_estimate,
        );
        assert_eq!(expect_deposit, precommits[i].pre_commit_deposit);
    }

    assert!(total_deposit.is_positive());
    assert_eq!(total_deposit, st.pre_commit_deposits);

    let expirations = h.collect_precommit_expirations(&rt, &st);
    let expected_precommit_expiration = st.quant_spec_every_deadline(&rt.policy).quantize_up(
        precommit_epoch
            + max_prove_commit_duration(&rt.policy, h.seal_proof_type).unwrap()
            + rt.policy.expired_pre_commit_clean_up_delay,
    );
    assert_eq!(HashMap::from([(expected_precommit_expiration, sector_no_as_uints)]), expirations);
}

mod miner_actor_precommit_batch {
    use super::*;

    #[test]
    fn one_sector() {
        assert_simple_batch(1, TokenAmount::zero(), TokenAmount::zero(), &[], ExitCode::OK, "");
    }

    #[test]
    fn thirty_two_sectors() {
        assert_simple_batch(32, TokenAmount::zero(), TokenAmount::zero(), &[], ExitCode::OK, "");
    }

    #[test]
    fn max_sectors() {
        assert_simple_batch(256, TokenAmount::zero(), TokenAmount::zero(), &[], ExitCode::OK, "");
    }

    #[test]
    fn one_deal() {
        assert_simple_batch(
            3,
            TokenAmount::zero(),
            TokenAmount::zero(),
            &[DealSpec { size: 32 << 30, verified_size: 0, ids: vec![1] }],
            ExitCode::OK,
            "",
        );
    }

    #[test]
    fn many_deals() {
        assert_simple_batch(
            3,
            TokenAmount::zero(),
            TokenAmount::zero(),
            &[
                DealSpec { size: 32 << 30, verified_size: 0, ids: vec![1] },
                DealSpec { size: 0, verified_size: 32 << 30, ids: vec![1] },
                DealSpec { size: 16 << 30, verified_size: 16 << 30, ids: vec![1, 2] },
            ],
            ExitCode::OK,
            "",
        );
    }

    #[test]
    fn empty_batch() {
        assert_simple_batch(
            0,
            TokenAmount::zero(),
            TokenAmount::zero(),
            &[],
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "batch empty",
        );
    }

    #[test]
    fn too_many_sectors() {
        assert_simple_batch(
            Policy::default().pre_commit_sector_batch_max_size + 1,
            TokenAmount::zero(),
            TokenAmount::zero(),
            &[],
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "batch of 257 too large",
        );
    }

    #[test]
    fn insufficient_balance() {
        assert_simple_batch(
            10,
            TokenAmount::from_atto(-1),
            TokenAmount::zero(),
            &[],
            ExitCode::USR_INSUFFICIENT_FUNDS,
            "insufficient funds",
        );
    }

    #[test]
    fn one_bad_apple_ruins_batch() {
        // This test does not enumerate all the individual conditions that could cause a single precommit
        // to be rejected. Those are covered in the PreCommitSector tests, and we know that that
        // method is implemented in terms of a batch of one.

        let period_offset = ChainEpoch::from(100);

        let h = ActorHarness::new(period_offset);
        let mut rt = h.new_runtime();
        rt.set_balance(BIG_BALANCE.clone());
        rt.set_received(TokenAmount::zero());

        let precommit_epoch = period_offset + 1;
        rt.set_epoch(precommit_epoch);
        h.construct_and_verify(&mut rt);
        let dl_info = h.deadline(&rt);

        let sector_expiration =
            dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period;
        let sectors = vec![
            h.make_pre_commit_params(100, precommit_epoch - 1, sector_expiration, vec![]),
            h.make_pre_commit_params(101, precommit_epoch - 1, sector_expiration, vec![]),
            h.make_pre_commit_params(102, precommit_epoch - 1, rt.epoch, vec![]), // Expires too soon
        ];

        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "sector expiration",
            h.pre_commit_sector_batch(
                &mut rt,
                PreCommitSectorBatchParams { sectors },
                &PreCommitBatchConfig { sector_weights: vec![], first_for_miner: true },
                &TokenAmount::zero(),
            ),
        );
        rt.reset();
    }

    #[test]
    fn duplicate_sector_rejects_batch() {
        let period_offset = ChainEpoch::from(100);

        let h = ActorHarness::new(period_offset);
        let mut rt = h.new_runtime();
        rt.set_balance(BIG_BALANCE.clone());
        rt.set_received(TokenAmount::zero());

        let precommit_epoch = period_offset + 1;
        rt.set_epoch(precommit_epoch);
        h.construct_and_verify(&mut rt);
        let dl_info = h.deadline(&rt);

        let sector_expiration =
            dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period;
        let sectors = vec![
            h.make_pre_commit_params(100, precommit_epoch - 1, sector_expiration, vec![]),
            h.make_pre_commit_params(101, precommit_epoch - 1, sector_expiration, vec![]),
            h.make_pre_commit_params(100, precommit_epoch - 1, sector_expiration, vec![]), // Duplicated
        ];

        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "duplicate sector number 100",
            h.pre_commit_sector_batch(
                &mut rt,
                PreCommitSectorBatchParams { sectors },
                &PreCommitBatchConfig { sector_weights: vec![], first_for_miner: true },
                &TokenAmount::zero(),
            ),
        );
        rt.reset();
    }
}
