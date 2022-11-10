use fil_actor_miner_state_v9::pledge_penalty_for_continued_fault;
use fil_actor_miner_state_v9::power_for_sectors;

use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

use num_traits::Zero;

mod util;
use crate::util::*;

// an expriration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn declare_fault_pays_fee_at_window_post() {
    let big_rewards = TokenAmount::from_whole(1000);

    // Get sector into proving state
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&mut rt);
    let all_sectors =
        h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);
    let pwr = power_for_sectors(h.sector_size, &all_sectors);

    // add lots of funds so penalties come from vesting funds
    h.apply_rewards(&mut rt, big_rewards, TokenAmount::zero());

    // find deadline for sector
    let st = h.get_state(&rt);
    let (dl_idx, _) = st.find_sector(&rt.policy, &rt.store, all_sectors[0].sector_number).unwrap();

    // advance to first proving period and submit so we'll have time to declare the fault next cycle
    h.advance_and_submit_posts(&mut rt, &all_sectors);

    // Declare the sector as faulted
    h.declare_faults(&mut rt, &all_sectors);

    // faults are recorded in state
    let dl = h.get_deadline(&rt, dl_idx);
    assert_eq!(pwr, dl.faulty_power);

    // Skip to end of proving period.
    let mut dl_info = h.deadline(&rt);
    while dl_info.index != dl_idx {
        dl_info = h.advance_deadline(&mut rt, CronConfig::default());
    }

    // faults are charged at ongoing rate and no additional power is removed
    let ongoing_pwr = power_for_sectors(h.sector_size, &all_sectors);
    let ongoing_penalty = pledge_penalty_for_continued_fault(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &ongoing_pwr.qa,
    );
    h.advance_deadline(
        &mut rt,
        CronConfig { continued_faults_penalty: ongoing_penalty, ..Default::default() },
    );
    h.check_state(&rt);
}
