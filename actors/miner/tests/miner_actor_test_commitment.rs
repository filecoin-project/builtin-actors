use fil_actor_miner::{
    max_prove_commit_duration, pre_commit_deposit_for_power, qa_power_for_weight, State,
};
use fil_actors_runtime::test_utils::*;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, SectorSize, MAX_SECTOR_NUMBER};

use num_traits::{FromPrimitive, Zero};

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
            deal_space: SectorSize::from_u64(deal_space),
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
            h.pre_commit_sector(&mut rt, precommit_params, util::PreCommitConfig::empty(), true),
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
                deal_space: Some(SectorSize::_64GiB),
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
}
