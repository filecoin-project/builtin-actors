use fvm_shared::{
    bigint::{BigInt, Zero},
    clock::ChainEpoch,
    econ::TokenAmount,
    error::ExitCode,
};

use fil_actor_miner::{max_prove_commit_duration, VestSpec};
use fil_actors_runtime::reward::FilterEstimate;
use fil_actors_runtime::test_utils::expect_abort;
use util::*;

mod util;

// an expiration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn prove_commit_aborts_if_pledge_requirement_not_met() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());

    h.construct_and_verify(&rt);

    // Set the circulating supply high and expected reward low in order to coerce
    // pledge requirements (BR + share of money supply, but capped at 1FIL)
    // to exceed pre-commit deposit (BR only).
    rt.set_circulating_supply(TokenAmount::from_whole(100_000_000));
    h.epoch_reward_smooth = FilterEstimate::new(BigInt::from(1e15 as u64), BigInt::zero());

    // prove one sector to establish collateral and locked funds
    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);

    // precommit another sector so we may prove it
    let expiration = DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period + PERIOD_OFFSET - 1;
    let precommit_epoch = *rt.epoch.borrow() + 1;
    rt.set_epoch(precommit_epoch);
    let params =
        h.make_pre_commit_params(h.next_sector_no, *rt.epoch.borrow() - 1, expiration, vec![]);
    let precommit = h.pre_commit_sector_and_get(&rt, params, PreCommitConfig::default(), false);

    // Confirm the unlocked PCD will not cover the new IP
    assert!(sectors[0].initial_pledge > precommit.pre_commit_deposit);

    // Set balance to exactly cover locked funds.
    let st = h.get_state(&rt);
    rt.balance.replace(&st.pre_commit_deposits + &st.initial_pledge + &st.locked_funds);

    rt.set_epoch(
        precommit_epoch + max_prove_commit_duration(&rt.policy, h.seal_proof_type).unwrap() - 1,
    );
    expect_abort(
        ExitCode::USR_INSUFFICIENT_FUNDS,
        h.deprecated_sector_commit(
            &rt,
            &vec![],
            h.make_prove_commit_params(h.next_sector_no),
            ProveCommitConfig::empty(),
        ),
    );
    rt.reset();

    // succeeds with enough free balance (enough to cover 2x IP)
    rt.balance.replace(
        &st.pre_commit_deposits + &st.initial_pledge + &st.initial_pledge + &st.locked_funds,
    );
    h.deprecated_sector_commit(
        &rt,
        &vec![],
        h.make_prove_commit_params(h.next_sector_no),
        ProveCommitConfig::empty(),
    )
    .unwrap();
    h.check_state(&rt);
}

#[test]
fn prove_commit_just_after_period_start_permits_post() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());

    // Epoch PERIOD_OFFSET+1 should be at the beginning of the miner's proving period so there will be time to commit
    // and PoSt a sector.
    rt.set_epoch(PERIOD_OFFSET + 1);
    h.construct_and_verify(&rt);

    // Commit a sector the very next epoch
    rt.set_epoch(PERIOD_OFFSET + 2);
    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);

    // advance cron to activate power.
    h.advance_and_submit_posts(&rt, &[sectors.first().unwrap().clone()]);
    h.check_state(&rt);
}

#[test]
fn verify_proof_does_not_vest_funds() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.balance.replace(BIG_BALANCE.clone());

    let precommit_epoch = PERIOD_OFFSET + 1;
    rt.set_epoch(precommit_epoch);

    h.construct_and_verify(&rt);
    let deadline = h.deadline(&rt);

    // Make a good commitment for the proof to target.
    let sector_no = 100;
    let params = h.make_pre_commit_params(
        sector_no,
        precommit_epoch - 1,
        deadline.period_end() + DEFAULT_SECTOR_EXPIRATION * rt.policy.wpost_proving_period,
        vec![],
    );
    h.pre_commit_sector_and_get(&rt, params, PreCommitConfig::default(), true);

    // add 1000 tokens that vest immediately
    let mut st = h.get_state(&rt);
    let _ = st
        .add_locked_funds(
            &rt.store,
            *rt.epoch.borrow(),
            &TokenAmount::from_atto(1000),
            &VestSpec { initial_delay: 0, vest_period: 1, step_duration: 1, quantization: 1 },
        )
        .unwrap();
    rt.replace_state(&st);

    rt.set_epoch(precommit_epoch + rt.policy.pre_commit_challenge_delay + 1);
    rt.balance.replace(TokenAmount::from_whole(1000));

    let prove_commit = h.make_prove_commit_params(sector_no);
    // The below call expects exactly the pledge delta for the proven sector, zero for any other vesting.
    h.deprecated_sector_commit(&rt, &vec![], prove_commit, ProveCommitConfig::empty()).unwrap();
}
