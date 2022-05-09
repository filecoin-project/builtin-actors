use fil_actor_miner::{pre_commit_deposit_for_power, qa_power_for_weight, State};
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{RegisteredSealProof, SectorNumber, SectorSize, MAX_SECTOR_NUMBER};

use num_traits::{FromPrimitive, Zero};

use std::collections::HashMap;

mod util;
use util::*;

// an expriration ~10 days greater than effective min expiration taking into account 30 days max
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
    let precommit = h.pre_commit_sector(
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
            + rt.policy.max_prove_commit_duration[&h.seal_proof_type]
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
}
