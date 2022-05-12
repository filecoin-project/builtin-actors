use fil_actors_runtime::{test_utils::MockRuntime, EPOCHS_IN_DAY};
use fvm_shared::{bigint::BigInt, clock::ChainEpoch};

mod util;
use num_traits::Zero;
use util::*;

#[test]
fn valid_precommits_then_aggregate_provecommit() {
    let period_offset = ChainEpoch::from(100);
    let precommit_challenge_delay = ChainEpoch::from(150);
    let wpost_proving_period = ChainEpoch::from(EPOCHS_IN_DAY);

    let actor = ActorHarness::new(period_offset);
    let mut rt = MockRuntime::default();
    let precommit_epoch = period_offset + 1;
    rt.set_epoch(precommit_epoch);
    actor.construct_and_verify(&mut rt);
    let dl_info = actor.deadline(&rt);

    // make a good commitment for the proof to target

    let prove_commit_epoch = precommit_epoch + precommit_challenge_delay + 1;
    // something on deadline boundary but > 180 days
    let expiration = dl_info.period_end() + wpost_proving_period;
    // fill the sector with verified seals
    let sector_weight = actor.sector_size as i64 * (expiration - prove_commit_epoch);
    let deal_weight = BigInt::zero();
    let verified_deal_weight = sector_weight;
}
