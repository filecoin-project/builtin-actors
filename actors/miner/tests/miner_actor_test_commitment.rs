use fil_actor_miner::{
    max_prove_commit_duration, pre_commit_deposit_for_power, qa_power_for_weight, DeadlineInfo,
    State, VestSpec,
};
use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::test_utils::*;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::consensus::{ConsensusFault, ConsensusFaultType};
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, MAX_SECTOR_NUMBER};

use num_traits::Zero;

use std::collections::HashMap;

mod util;
use util::*;

// an expiration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: i64 = 220;

// A balance for use in tests where the miner's low balance is not interesting.
const BIG_BALANCE: u128 = 1_000_000_000_000_000_000_000_000u128;

fn assert_simple_pre_commit(
    sector_number: SectorNumber,
    deal_size: u64,
    verified_deal_size: u64,
    deal_ids: &[DealID],
) {
    let period_offset = ChainEpoch::from(100);

    let mut h = ActorHarness::new(period_offset);
    h.set_proof_type(RegisteredSealProof::StackedDRG64GiBV1);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    rt.set_received(TokenAmount::zero());

    let precommit_epoch = period_offset + 1;
    rt.set_epoch(precommit_epoch);
    h.construct_and_verify(&mut rt);
    let dl_info = h.deadline(&rt);

    let expiration =
        dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period; // on deadline boundary but > 180 days
    let prove_commit_epoch = precommit_epoch + rt.policy.pre_commit_challenge_delay + 1;
    let deal_lifespan = (expiration - prove_commit_epoch) as u64;
    let deal_space = deal_size + verified_deal_size;
    let deal_weight = deal_size * deal_lifespan;
    let verified_deal_weight = verified_deal_size * deal_lifespan;

    let precommit_params =
        h.make_pre_commit_params(sector_number, precommit_epoch - 1, expiration, deal_ids.to_vec());
    let precommit = h.pre_commit_sector_and_get(
        &mut rt,
        precommit_params.clone(),
        util::PreCommitConfig {
            deal_weight: BigInt::from(deal_weight),
            verified_deal_weight: BigInt::from(verified_deal_weight),
            deal_space,
        },
        true,
    );

    // Check precommit expectations.
    assert_eq!(precommit_epoch, precommit.pre_commit_epoch);
    assert_eq!(BigInt::from(deal_weight), precommit.deal_weight);
    assert_eq!(BigInt::from(verified_deal_weight), precommit.verified_deal_weight);

    assert_eq!(sector_number, precommit.info.sector_number);
    assert_eq!(precommit_params.seal_proof, precommit.info.seal_proof);
    assert_eq!(precommit_params.sealed_cid, precommit.info.sealed_cid);
    assert_eq!(precommit_params.seal_rand_epoch, precommit.info.seal_rand_epoch);
    assert_eq!(precommit_params.deal_ids, precommit.info.deal_ids);
    assert_eq!(precommit_params.expiration, precommit.info.expiration);

    let pwr_estimate = qa_power_for_weight(
        h.sector_size,
        precommit.info.expiration - precommit_epoch,
        &BigInt::from(deal_weight),
        &BigInt::from(verified_deal_weight),
    );
    let expected_deposit = pre_commit_deposit_for_power(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &pwr_estimate,
    );
    assert_eq!(expected_deposit, precommit.pre_commit_deposit);

    let st: State = rt.get_state();
    assert!(expected_deposit > TokenAmount::default());
    assert_eq!(expected_deposit, st.pre_commit_deposits);

    let expirations = h.collect_precommit_expirations(&rt, &st);
    let expected_precommit_expiration = st.quant_spec_every_deadline(&rt.policy).quantize_up(
        precommit_epoch
            + max_prove_commit_duration(&rt.policy, h.seal_proof_type).unwrap()
            + rt.policy.expired_pre_commit_clean_up_delay,
    );
    assert_eq!(HashMap::from([(expected_precommit_expiration, vec![sector_number])]), expirations);
}

mod miner_actor_test_commitment {
    use super::*;

    #[test]
    fn no_deals() {
        assert_simple_pre_commit(0, 0, 0, &[]);
    }

    #[test]
    fn max_sector_number() {
        assert_simple_pre_commit(MAX_SECTOR_NUMBER, 0, 0, &[]);
    }

    #[test]
    fn unverified_deal() {
        assert_simple_pre_commit(100, 32 << 30, 0, &[1]);
    }

    #[test]
    fn verified_deal() {
        assert_simple_pre_commit(100, 0, 32 << 30, &[1]);
    }

    #[test]
    fn two_deals() {
        assert_simple_pre_commit(100, 16 << 30, 16 << 30, &[1, 2]);
    }

    #[test]
    fn insufficient_funds_for_pre_commit() {
        let period_offset = ChainEpoch::from(100);
        let insufficient_balance = TokenAmount::from(10u8); // 10 AttoFIL

        let mut h = ActorHarness::new(period_offset);
        h.set_proof_type(RegisteredSealProof::StackedDRG64GiBV1);
        let mut rt = h.new_runtime();
        rt.set_balance(insufficient_balance);
        rt.set_received(TokenAmount::zero());

        let precommit_epoch = period_offset + 1;
        rt.set_epoch(precommit_epoch);
        h.construct_and_verify(&mut rt);
        let deadline = h.deadline(&rt);
        let challenge_epoch = precommit_epoch - 1;
        let expiration =
            deadline.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period;

        let precommit_params = h.make_pre_commit_params(101, challenge_epoch, expiration, vec![]);

        expect_abort(
            ExitCode::USR_INSUFFICIENT_FUNDS,
            h.pre_commit_sector(&mut rt, precommit_params, util::PreCommitConfig::default(), true),
        );
        util::check_state_invariants(&rt);
    }

    #[test]
    fn deal_space_exceeds_sector_space() {
        let period_offset = ChainEpoch::from(100);

        let mut h = ActorHarness::new(period_offset);
        h.set_proof_type(RegisteredSealProof::StackedDRG32GiBV1);
        let mut rt = h.new_runtime();
        rt.set_balance(TokenAmount::from(BIG_BALANCE));
        rt.set_received(TokenAmount::zero());

        let precommit_epoch = period_offset + 1;
        rt.set_epoch(precommit_epoch);
        h.construct_and_verify(&mut rt);
        let deadline = h.deadline(&rt);
        let challenge_epoch = precommit_epoch - 1;
        let expiration =
            deadline.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period;

        let precommit_params = h.make_pre_commit_params(101, challenge_epoch, expiration, vec![1]);

        let ret = h.pre_commit_sector(
            &mut rt,
            precommit_params,
            util::PreCommitConfig {
                deal_weight: BigInt::default(),
                verified_deal_weight: BigInt::default(),
                deal_space: h.sector_size as u64 + 1,
            },
            true,
        );
        expect_abort_contains_message(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            "deals too large to fit in sector",
            ret,
        );
        util::check_state_invariants(&rt);
    }

    #[test]
    fn precommit_pays_back_fee_debt() {
        let period_offset = ChainEpoch::from(100);

        let mut h = ActorHarness::new(period_offset);
        h.set_proof_type(RegisteredSealProof::StackedDRG64GiBV1);
        let mut rt = h.new_runtime();
        rt.set_balance(TokenAmount::from(BIG_BALANCE));
        rt.set_received(TokenAmount::zero());

        let precommit_epoch = period_offset + 1;
        rt.set_epoch(precommit_epoch);
        h.construct_and_verify(&mut rt);
        let deadline = h.deadline(&rt);
        let challenge_epoch = precommit_epoch - 1;
        let expiration =
            deadline.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period;

        let mut st: State = rt.get_state();
        st.fee_debt = TokenAmount::from(9999);
        rt.replace_state(&st);

        let precommit_params = h.make_pre_commit_params(101, challenge_epoch, expiration, vec![1]);

        h.pre_commit_sector(&mut rt, precommit_params, util::PreCommitConfig::default(), true)
            .unwrap();
        let st: State = rt.get_state();
        assert_eq!(TokenAmount::zero(), st.fee_debt);
        util::check_state_invariants(&rt);
    }

    #[test]
    fn invalid_pre_commit_rejected() {
        let period_offset = ChainEpoch::from(100);

        let mut h = ActorHarness::new(period_offset);
        let mut rt = h.new_runtime();
        rt.set_balance(TokenAmount::from(BIG_BALANCE));
        rt.set_received(TokenAmount::zero());

        let precommit_epoch = period_offset + 1;
        rt.set_epoch(precommit_epoch);
        h.construct_and_verify(&mut rt);
        let deadline = h.deadline(&rt);
        let challenge_epoch = precommit_epoch - 1;

        let old_sector =
            &h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true)
                [0];
        let st: State = rt.get_state();
        assert!(st.deadline_cron_active);

        let expiration =
            deadline.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period;

        // Good commitment.
        {
            let precommit_params =
                h.make_pre_commit_params(101, challenge_epoch, expiration, vec![]);
            h.pre_commit_sector_and_get(
                &mut rt,
                precommit_params.clone(),
                util::PreCommitConfig::default(),
                false,
            );
            // Duplicate pre-commit sector ID
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "already allocated", ret);
            rt.reset();
        }

        // Sector ID already committed
        {
            let precommit_params = h.make_pre_commit_params(
                old_sector.sector_number,
                challenge_epoch,
                expiration,
                vec![],
            );
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "already allocated", ret);
            rt.reset();
        }

        // Bad sealed CID
        {
            let mut precommit_params =
                h.make_pre_commit_params(102, challenge_epoch, deadline.period_end(), vec![]);
            precommit_params.sealed_cid = make_cid("Random Data".as_bytes(), 0);
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "sealed CID had wrong prefix",
                ret,
            );
            rt.reset();
        }

        // Bad seal proof type
        {
            let mut precommit_params =
                h.make_pre_commit_params(102, challenge_epoch, deadline.period_end(), vec![]);
            precommit_params.seal_proof = RegisteredSealProof::StackedDRG8MiBV1;
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "unsupported seal proof type",
                ret,
            );
            rt.reset();
        }

        // Expires at current epoch
        {
            let precommit_params = h.make_pre_commit_params(102, challenge_epoch, rt.epoch, vec![]);
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "must be after activation",
                ret,
            );
            rt.reset();
        }

        // Expires before current epoch
        {
            let precommit_params =
                h.make_pre_commit_params(102, challenge_epoch, rt.epoch - 1, vec![]);
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "must be after activation",
                ret,
            );
            rt.reset();
        }

        // Expires too early
        {
            let early_expiration = rt.policy.min_sector_expiration - EPOCHS_IN_DAY;
            let precommit_params =
                h.make_pre_commit_params(102, challenge_epoch, early_expiration, vec![]);
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "must exceed", ret);
            rt.reset();
        }

        // Expires before min duration + max seal duration
        {
            let expiration = rt.epoch
                + rt.policy.min_sector_expiration
                + max_prove_commit_duration(&rt.policy, h.seal_proof_type).unwrap()
                - 1;
            let precommit_params =
                h.make_pre_commit_params(102, challenge_epoch, expiration, vec![]);
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "must exceed", ret);
            rt.reset();
        }

        // Errors when expiry too far in the future
        {
            rt.set_epoch(precommit_epoch);
            let expiration = deadline.period_end()
                + rt.policy.wpost_proving_period
                    * (rt.policy.max_sector_expiration_extension / rt.policy.wpost_proving_period
                        + 1);
            let precommit_params =
                h.make_pre_commit_params(102, challenge_epoch, expiration, vec![]);
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "invalid expiration",
                ret,
            );
            rt.reset();
        }

        // Errors when expiry too far in the future (bis)
        {
            rt.set_epoch(precommit_epoch);
            let expiration = rt.epoch + rt.policy.max_sector_expiration_extension + 1;
            let precommit_params =
                h.make_pre_commit_params(102, challenge_epoch, expiration, vec![]);
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "invalid expiration",
                ret,
            );
            rt.reset();
        }

        // Sector ID out of range
        {
            let precommit_params = h.make_pre_commit_params(
                MAX_SECTOR_NUMBER + 1,
                challenge_epoch,
                expiration,
                vec![],
            );
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "out of range", ret);
            rt.reset();
        }

        // Seal randomness challenge too far in past
        {
            let too_old_challenge_epoch = precommit_epoch
                - rt.policy.chain_finality
                - max_prove_commit_duration(&rt.policy, h.seal_proof_type).unwrap()
                - 1;
            let precommit_params =
                h.make_pre_commit_params(102, too_old_challenge_epoch, expiration, vec![]);
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "too old", ret);
            rt.reset();
        }

        // Deals too large for sector
        {
            let deal_weight = BigInt::from(32u32 << 30) * (expiration - rt.epoch);
            let precommit_params =
                h.make_pre_commit_params(0, challenge_epoch, expiration, vec![1]);
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig {
                    deal_weight,
                    verified_deal_weight: BigInt::zero(),
                    deal_space: h.sector_size as u64 + 1,
                },
                false,
            );
            expect_abort_contains_message(ExitCode::USR_ILLEGAL_ARGUMENT, "deals too large", ret);
        }

        // Try to precommit while in fee debt with insufficient balance
        {
            let mut st: State = rt.get_state();
            st.fee_debt =
                rt.balance.borrow().clone() + TokenAmount::from(10_000_000_000_000_000_000i128);
            rt.replace_state(&st);
            let precommit_params =
                h.make_pre_commit_params(102, challenge_epoch, expiration, vec![]);
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(
                ExitCode::USR_INSUFFICIENT_FUNDS,
                "unlocked balance can not repay fee debt",
                ret,
            );
            // reset state back to normal
            st.fee_debt = TokenAmount::zero();
            rt.replace_state(&st);
            rt.reset();
        }

        // Try to precommit with an active consensus fault
        {
            let st: State = rt.get_state();
            let fault = ConsensusFault {
                target: h.receiver,
                epoch: rt.epoch - 1,
                fault_type: ConsensusFaultType::DoubleForkMining,
            };
            let test_addr = Address::new_actor(b"satoshi");
            h.report_consensus_fault(&mut rt, test_addr, Some(fault)).unwrap();
            let precommit_params =
                h.make_pre_commit_params(102, challenge_epoch, expiration, vec![]);
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                false,
            );
            expect_abort_contains_message(ExitCode::USR_FORBIDDEN, "active consensus fault", ret);
            // reset state back to normal
            rt.replace_state(&st);
            rt.reset();
        }
    }

    #[test]
    fn fails_with_too_many_deals() {
        let setup = |proof: RegisteredSealProof| -> (MockRuntime, ActorHarness, DeadlineInfo) {
            let period_offset = ChainEpoch::from(100);

            let mut h = ActorHarness::new(period_offset);
            h.set_proof_type(proof);
            let mut rt = h.new_runtime();
            rt.set_balance(TokenAmount::from(BIG_BALANCE));
            rt.set_received(TokenAmount::zero());

            rt.set_epoch(period_offset + 1);
            h.construct_and_verify(&mut rt);
            let deadline = h.deadline(&rt);
            (rt, h, deadline)
        };

        let make_deal_ids = |n| -> Vec<DealID> { (0..n).collect() };

        let sector_number: SectorNumber = 100;
        let deal_limits = [
            (RegisteredSealProof::StackedDRG2KiBV1P1, 256),
            (RegisteredSealProof::StackedDRG32GiBV1P1, 256),
            (RegisteredSealProof::StackedDRG64GiBV1P1, 512),
        ];

        for (proof, limit) in deal_limits {
            // attempt to pre-commmit a sector with too many deals
            let (mut rt, h, deadline) = setup(proof);
            let expiration =
                deadline.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period;
            let precommit_params = h.make_pre_commit_params(
                sector_number,
                rt.epoch - 1,
                expiration,
                make_deal_ids(limit + 1),
            );
            let ret = h.pre_commit_sector(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                true,
            );
            expect_abort_contains_message(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                "too many deals for sector",
                ret,
            );

            // sector at or below limit succeeds
            let (mut rt, h, _) = setup(proof);
            let precommit_params = h.make_pre_commit_params(
                sector_number,
                rt.epoch - 1,
                expiration,
                make_deal_ids(limit),
            );
            h.pre_commit_sector_and_get(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                true,
            );
            util::check_state_invariants(&rt);
        }
    }

    #[test]
    fn precommit_checks_seal_proof_version() {
        let period_offset = ChainEpoch::from(100);

        let h = ActorHarness::new(period_offset);
        let mut rt = h.new_runtime();
        rt.set_balance(TokenAmount::from(BIG_BALANCE));
        rt.set_received(TokenAmount::zero());

        h.construct_and_verify(&mut rt);
        let precommit_epoch = period_offset + 1;
        rt.set_epoch(precommit_epoch);
        let deadline = h.deadline(&rt);
        let challenge_epoch = precommit_epoch - 1;
        let expiration =
            deadline.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period;
        {
            let mut precommit_params =
                h.make_pre_commit_params(104, challenge_epoch, expiration, vec![]);
            precommit_params.seal_proof = RegisteredSealProof::StackedDRG32GiBV1;
            expect_abort(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                h.pre_commit_sector(
                    &mut rt,
                    precommit_params.clone(),
                    util::PreCommitConfig::default(),
                    true,
                ),
            );
            rt.reset();
            precommit_params.seal_proof = RegisteredSealProof::StackedDRG32GiBV1P1;
            h.pre_commit_sector_and_get(
                &mut rt,
                precommit_params,
                util::PreCommitConfig::default(),
                true,
            );
        }

        util::check_state_invariants(&rt);
    }

    #[test]
    fn precommit_does_not_vest_funds() {
        let period_offset = ChainEpoch::from(100);

        let mut h = ActorHarness::new(period_offset);
        h.set_proof_type(RegisteredSealProof::StackedDRG32GiBV1P1);
        let mut rt = h.new_runtime();
        rt.set_balance(TokenAmount::from(BIG_BALANCE));
        rt.set_received(TokenAmount::zero());
        let precommit_epoch = period_offset + 1;
        rt.set_epoch(precommit_epoch);
        h.construct_and_verify(&mut rt);
        let dl_info = h.deadline(&rt);

        // Make a good commitment for the proof to target.
        let sector_number: SectorNumber = 100;
        let expiration =
            dl_info.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period; // something on deadline boundary but > 180 days

        // add 1000 tokens that vest immediately
        let mut st: State = rt.get_state();
        let _ = st
            .add_locked_funds(
                &rt.store,
                rt.epoch,
                &TokenAmount::from(1000u16),
                &VestSpec { initial_delay: 0, vest_period: 1, step_duration: 1, quantization: 1 },
            )
            .unwrap();
        rt.replace_state(&st);

        rt.set_epoch(rt.epoch + 2);

        // Pre-commit with a deal in order to exercise non-zero deal weights.
        let precommit_params =
            h.make_pre_commit_params(sector_number, precommit_epoch - 1, expiration, vec![1]);
        // The below call expects no pledge delta.
        h.pre_commit_sector_and_get(
            &mut rt,
            precommit_params,
            util::PreCommitConfig::default(),
            true,
        );
    }
}
