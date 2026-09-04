use fil_actor_miner::{
    PreCommitSectorBatchParams, PreCommitSectorParams, State, max_prove_commit_duration,
    pre_commit_deposit_for_power, qa_power_max,
};
use fil_actors_runtime::test_utils::*;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::SectorNumber;

use num_traits::Zero;

use cid::Cid;
use std::collections::HashMap;

mod util;

use util::*;

// an expiration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: i64 = 220;

// A balance for use in tests where the miner's low balance is not interesting.

#[derive(Default, Clone)]
struct DealSpec {
    ids: Vec<DealID>,
    commd: Option<Cid>,
}

fn assert_simple_batch(
    batch_size: usize,
    balance_surplus: TokenAmount,
    deal_specs: &[DealSpec],
    exit_code: ExitCode,
    error_str: &str,
) {
    let period_offset = ChainEpoch::from(100);

    let h = ActorHarness::new_with_options(HarnessOptions { proving_period_offset: period_offset });
    let rt = h.new_runtime();

    let precommit_epoch = period_offset + 1;
    rt.set_epoch(precommit_epoch);
    h.construct_and_verify(&rt);
    let dl_info = h.deadline(&rt);

    let sector_nos: Vec<SectorNumber> = (0..batch_size).map(|x| x as u64 + 100).collect();
    let sector_no_as_uints: Vec<u64> = (0..batch_size).map(|x| x as u64 + 100).collect();
    let sector_expiration =
        dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period; // on deadline boundary but > 180 days

    let mut sectors = vec![PreCommitSectorParams::default(); batch_size];
    let mut conf =
        PreCommitBatchConfig { sector_unsealed_cid: vec![None; batch_size], first_for_miner: true };
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

        conf.sector_unsealed_cid[i] = deals.commd;
        let pwr_estimate = qa_power_max(h.sector_size);
        deposits[i] = pre_commit_deposit_for_power(
            &h.epoch_reward_smooth,
            &h.epoch_qa_power_smooth,
            &pwr_estimate,
        );
    }
    let total_deposit: TokenAmount = deposits.iter().sum();
    rt.set_balance(&total_deposit + balance_surplus);

    if exit_code != ExitCode::OK {
        expect_abort_contains_message(
            exit_code,
            error_str,
            h.pre_commit_sector_batch(&rt, PreCommitSectorBatchParams { sectors }, &conf),
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
        &rt,
        PreCommitSectorBatchParams { sectors: sectors.clone() },
        &conf,
    );

    // Check precommits
    let st: State = rt.get_state();
    for i in 0..batch_size {
        assert_eq!(precommit_epoch, precommits[i].pre_commit_epoch);
        assert_eq!(conf.sector_unsealed_cid[i], precommits[i].info.unsealed_cid.0);

        assert_eq!(sector_nos[i], precommits[i].info.sector_number);

        let pwr_estimate = qa_power_max(h.sector_size);
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
        assert_simple_batch(1, TokenAmount::zero(), &[], ExitCode::OK, "");
    }

    #[test]
    fn thirty_two_sectors() {
        assert_simple_batch(32, TokenAmount::zero(), &[], ExitCode::OK, "");
    }

    #[test]
    fn max_sectors() {
        assert_simple_batch(256, TokenAmount::zero(), &[], ExitCode::OK, "");
    }

    #[test]
    fn deals_rejected() {
        assert_simple_batch(
            1,
            TokenAmount::zero(),
            &[DealSpec { ids: vec![1], commd: None }],
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "pre-committed deals are no longer supported",
        );
    }

    #[test]
    fn empty_batch() {
        assert_simple_batch(
            0,
            TokenAmount::zero(),
            &[],
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "batch empty",
        );
    }

    #[test]
    fn insufficient_balance() {
        assert_simple_batch(
            10,
            TokenAmount::from_atto(-1),
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

        let h =
            ActorHarness::new_with_options(HarnessOptions { proving_period_offset: period_offset });

        let rt = h.new_runtime();

        rt.set_balance(BIG_BALANCE.clone());
        rt.set_received(TokenAmount::zero());

        let precommit_epoch = period_offset + 1;
        rt.set_epoch(precommit_epoch);
        h.construct_and_verify(&rt);
        let dl_info = h.deadline(&rt);

        let sector_expiration =
            dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period;
        let sectors = vec![
            h.make_pre_commit_params(100, precommit_epoch - 1, sector_expiration, vec![]),
            h.make_pre_commit_params(101, precommit_epoch - 1, sector_expiration, vec![]),
            h.make_pre_commit_params(102, precommit_epoch - 1, *rt.epoch.borrow(), vec![]), // Expires too soon
        ];

        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "sector expiration",
            h.pre_commit_sector_batch(
                &rt,
                PreCommitSectorBatchParams { sectors },
                &PreCommitBatchConfig { sector_unsealed_cid: vec![], first_for_miner: true },
            ),
        );
        rt.reset();
    }

    #[test]
    fn duplicate_sector_rejects_batch() {
        let period_offset = ChainEpoch::from(100);

        let h =
            ActorHarness::new_with_options(HarnessOptions { proving_period_offset: period_offset });
        let rt = h.new_runtime();

        rt.set_balance(BIG_BALANCE.clone());
        rt.set_received(TokenAmount::zero());

        let precommit_epoch = period_offset + 1;
        rt.set_epoch(precommit_epoch);
        h.construct_and_verify(&rt);
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
                &rt,
                PreCommitSectorBatchParams { sectors },
                &PreCommitBatchConfig { sector_unsealed_cid: vec![], first_for_miner: true },
            ),
        );
        rt.reset();
    }
}
