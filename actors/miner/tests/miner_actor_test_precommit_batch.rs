use fil_actor_market::{Method as MarketMethod, SectorDealData};
use fil_actor_miner::{
    aggregate_pre_commit_network_fee, max_prove_commit_duration, pre_commit_deposit_for_power,
    qa_power_max, PreCommitSectorBatchParams, PreCommitSectorParams, State,
};
use fil_actor_power::Method as PowerMethod;
use fil_actors_runtime::runtime::Policy;
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
    v2: bool,
    batch_size: usize,
    balance_surplus: TokenAmount,
    base_fee: TokenAmount,
    deal_specs: &[DealSpec],
    exit_code: ExitCode,
    error_str: &str,
) {
    let period_offset = ChainEpoch::from(100);

    let h = ActorHarness::new_with_options(HarnessOptions {
        use_v2_pre_commit_and_replica_update: v2,
        proving_period_offset: period_offset,
    });
    let mut rt = h.new_runtime();

    let precommit_epoch = period_offset + 1;
    rt.set_epoch(precommit_epoch);
    h.construct_and_verify(&mut rt);
    let dl_info = h.deadline(&rt);

    let sector_nos: Vec<SectorNumber> = (0..batch_size).map(|x| x as u64 + 100).collect();
    let sector_no_as_uints: Vec<u64> = (0..batch_size).map(|x| x as u64 + 100).collect();
    let sector_expiration =
        dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period; // on deadline boundary but > 180 days

    let mut sectors = vec![PreCommitSectorParams::default(); batch_size];
    let mut conf = PreCommitBatchConfig {
        sector_deal_data: vec![SectorDealData::default(); batch_size],
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

        conf.sector_deal_data[i] = SectorDealData { commd: deals.commd };
        let pwr_estimate = qa_power_max(h.sector_size);
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
        assert_eq!(conf.sector_deal_data[i].commd, precommits[i].info.unsealed_cid.0);

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
    use fil_actor_market::{
        SectorDeals, VerifyDealsForActivationParams, VerifyDealsForActivationReturn,
    };
    use fil_actor_miner::{
        new_deadline_info_from_offset_and_epoch, Actor, Method, PreCommitSectorBatchParams2,
    };
    use fil_actors_runtime::{STORAGE_MARKET_ACTOR_ADDR, STORAGE_POWER_ACTOR_ADDR};
    use fvm_ipld_encoding::RawBytes;
    use test_case::test_case;

    #[test_case(false; "v1")]
    #[test_case(true; "v2")]
    fn one_sector(v2: bool) {
        assert_simple_batch(v2, 1, TokenAmount::zero(), TokenAmount::zero(), &[], ExitCode::OK, "");
    }

    #[test_case(false; "v1")]
    #[test_case(true; "v2")]
    fn thirty_two_sectors(v2: bool) {
        assert_simple_batch(
            v2,
            32,
            TokenAmount::zero(),
            TokenAmount::zero(),
            &[],
            ExitCode::OK,
            "",
        );
    }

    #[test_case(false; "v1")]
    #[test_case(true; "v2")]
    fn max_sectors(v2: bool) {
        assert_simple_batch(
            v2,
            256,
            TokenAmount::zero(),
            TokenAmount::zero(),
            &[],
            ExitCode::OK,
            "",
        );
    }

    #[test_case(false; "v1")]
    #[test_case(true; "v2")]
    fn one_deal(v2: bool) {
        assert_simple_batch(
            v2,
            3,
            TokenAmount::zero(),
            TokenAmount::zero(),
            &[DealSpec { ids: vec![1], commd: Some(make_piece_cid("1".as_bytes())) }],
            ExitCode::OK,
            "",
        );
    }

    #[test_case(false; "v1")]
    #[test_case(true; "v2")]
    fn many_deals(v2: bool) {
        assert_simple_batch(
            v2,
            3,
            TokenAmount::zero(),
            TokenAmount::zero(),
            &[
                DealSpec { ids: vec![1], commd: Some(make_piece_cid("1".as_bytes())) },
                DealSpec { ids: vec![2], commd: Some(make_piece_cid("2".as_bytes())) },
                DealSpec { ids: vec![1, 2], commd: Some(make_piece_cid("1|2".as_bytes())) },
            ],
            ExitCode::OK,
            "",
        );
    }

    #[test_case(false; "v1")]
    #[test_case(true; "v2")]
    fn empty_batch(v2: bool) {
        assert_simple_batch(
            v2,
            0,
            TokenAmount::zero(),
            TokenAmount::zero(),
            &[],
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "batch empty",
        );
    }

    #[test_case(false; "v1")]
    #[test_case(true; "v2")]
    fn too_many_sectors(v2: bool) {
        assert_simple_batch(
            v2,
            Policy::default().pre_commit_sector_batch_max_size + 1,
            TokenAmount::zero(),
            TokenAmount::zero(),
            &[],
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "batch of 257 too large",
        );
    }

    #[test_case(false; "v1")]
    #[test_case(true; "v2")]
    fn insufficient_balance(v2: bool) {
        assert_simple_batch(
            v2,
            10,
            TokenAmount::from_atto(-1),
            TokenAmount::zero(),
            &[],
            ExitCode::USR_INSUFFICIENT_FUNDS,
            "insufficient funds",
        );
    }

    #[test_case(false; "v1")]
    #[test_case(true; "v2")]
    fn one_bad_apple_ruins_batch(v2: bool) {
        // This test does not enumerate all the individual conditions that could cause a single precommit
        // to be rejected. Those are covered in the PreCommitSector tests, and we know that that
        // method is implemented in terms of a batch of one.

        let period_offset = ChainEpoch::from(100);

        let h = ActorHarness::new_with_options(HarnessOptions {
            use_v2_pre_commit_and_replica_update: v2,
            proving_period_offset: period_offset,
        });

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
                &PreCommitBatchConfig { sector_deal_data: vec![], first_for_miner: true },
                &TokenAmount::zero(),
            ),
        );
        rt.reset();
    }

    #[test_case(false; "v1")]
    #[test_case(true; "v2")]
    fn duplicate_sector_rejects_batch(v2: bool) {
        // This test does not enumerate all the individual conditions that could cause a single precommit
        // to be rejected. Those are covered in the PreCommitSector tests, and we know that that
        // method is implemented in terms of a batch of one.

        let period_offset = ChainEpoch::from(100);

        let h = ActorHarness::new_with_options(HarnessOptions {
            use_v2_pre_commit_and_replica_update: v2,
            proving_period_offset: period_offset,
        });
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
                &PreCommitBatchConfig { sector_deal_data: vec![], first_for_miner: true },
                &TokenAmount::zero(),
            ),
        );
        rt.reset();
    }

    #[test]
    fn mismatch_of_commd() {
        let period_offset = ChainEpoch::from(100);

        let h = ActorHarness::new_with_options(HarnessOptions {
            use_v2_pre_commit_and_replica_update: true,
            proving_period_offset: period_offset,
        });
        let mut rt = h.new_runtime();
        rt.set_balance(BIG_BALANCE.clone());
        rt.set_received(TokenAmount::zero());

        let precommit_epoch = period_offset + 1;
        rt.set_epoch(precommit_epoch);
        h.construct_and_verify(&mut rt);
        let dl_info = h.deadline(&rt);

        let sector_expiration =
            dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period;
        let sector = h.make_pre_commit_params_v2(
            100,
            precommit_epoch - 1,
            sector_expiration,
            vec![1],
            Some(make_piece_cid(&[1])),
        );
        let sectors = vec![sector];
        {
            rt.set_caller(*ACCOUNT_ACTOR_CODE_ID, h.worker);
            rt.expect_validate_caller_addr(h.caller_addrs());

            h.expect_query_network_info(&mut rt);
            let mut sector_deals = Vec::new();
            let mut sector_deal_data = Vec::new();
            let mut any_deals = false;
            for sector in &sectors {
                sector_deals.push(SectorDeals {
                    sector_type: sector.seal_proof,
                    sector_expiry: sector.expiration,
                    deal_ids: sector.deal_ids.clone(),
                });

                sector_deal_data.push(SectorDealData { commd: Some(make_piece_cid(&[2])) }); //mismatch here
                                                                                             // Sanity check on expectations
                let sector_has_deals = !sector.deal_ids.is_empty();
                any_deals |= sector_has_deals;
            }
            if any_deals {
                let vdparams = VerifyDealsForActivationParams { sectors: sector_deals };
                let vdreturn = VerifyDealsForActivationReturn { sectors: sector_deal_data };
                rt.expect_send(
                    *STORAGE_MARKET_ACTOR_ADDR,
                    MarketMethod::VerifyDealsForActivation as u64,
                    RawBytes::serialize(vdparams).unwrap(),
                    TokenAmount::zero(),
                    RawBytes::serialize(vdreturn).unwrap(),
                    ExitCode::OK,
                );
            }

            let state = h.get_state(&rt);

            let dlinfo = new_deadline_info_from_offset_and_epoch(
                &rt.policy,
                state.proving_period_start,
                rt.epoch,
            );
            let cron_params = make_deadline_cron_event_params(dlinfo.last());
            rt.expect_send(
                *STORAGE_POWER_ACTOR_ADDR,
                PowerMethod::EnrollCronEvent as u64,
                RawBytes::serialize(cron_params).unwrap(),
                TokenAmount::zero(),
                RawBytes::default(),
                ExitCode::OK,
            );

            let result = rt.call::<Actor>(
                Method::PreCommitSectorBatch2 as u64,
                &RawBytes::serialize(PreCommitSectorBatchParams2 { sectors }).unwrap(),
            );

            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "and passed CompactCommD",
                result,
            );
        }
    }
}
