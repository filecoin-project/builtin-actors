use fil_actor_miner::testing::{check_deadline_state_invariants, DeadlineStateSummary};
use fil_actor_miner::{
    pledge_penalty_for_continued_fault, power_for_sectors, Deadline, PowerPair, SectorOnChainInfo,
};
use fil_actors_runtime::test_utils::MockRuntime;
use fil_actors_runtime::MessageAccumulator;
use fvm_ipld_bitfield::BitField;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::{ChainEpoch, QuantSpec};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::SectorSize;
use std::ops::Neg;

mod util;
use crate::util::*;

// an expriration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

const PERIOD_OFFSET: ChainEpoch = 100;
const BIG_BALANCE: u128 = 1_000_000_000_000_000_000_000_000u128;
const BIG_REWARDS: u128 = 1_000 * 1e18 as u128;

#[test]
fn cron_on_inactive_state() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    let st = h.get_state(&rt);
    assert_eq!(PERIOD_OFFSET - rt.policy.wpost_proving_period, st.proving_period_start);
    assert!(!st.continue_deadline_cron());

    // cron does nothing and does not enroll another cron
    let deadline = h.deadline(&rt);
    rt.set_epoch(deadline.last());
    h.on_deadline_cron(&mut rt, CronConfig { no_enrollment: true, ..CronConfig::default() });

    h.check_state(&rt);
}

#[test]
fn sector_expires() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    let sectors =
        h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);
    // advance cron to activate power.
    h.advance_and_submit_posts(&mut rt, &sectors);
    let active_power = power_for_sectors(h.sector_size, &sectors);

    let mut st = h.get_state(&rt);
    let initial_pledge = &st.initial_pledge;
    let expiration_raw = sectors[0].expiration;
    assert!(st.deadline_cron_active);

    // setup state to simulate moving forward all the way to expiry
    let (dl_idx, _) = st.find_sector(&rt.policy, &rt.store, sectors[0].sector_number).unwrap();
    let exp_quant_spec = st.quant_spec_for_deadline(&rt.policy, dl_idx);
    let expiration = exp_quant_spec.quantize_up(expiration_raw);
    let remaining_epoch = expiration - st.proving_period_start;
    let remaining_period = remaining_epoch / rt.policy.wpost_proving_period + 1;
    st.proving_period_start += remaining_period * rt.policy.wpost_proving_period;
    st.current_deadline = dl_idx;
    rt.replace_state(&st);

    // Advance to expiration epoch and expect expiration during cron
    rt.set_epoch(expiration);
    let power_delta = active_power.neg();

    // because we skip forward in state the sector is detected faulty, no penalty
    h.advance_deadline(
        &mut rt,
        CronConfig {
            no_enrollment: true,
            expired_sectors_power_delta: Some(power_delta),
            expired_sectors_pledge_delta: initial_pledge.neg(),
            ..CronConfig::default()
        },
    );
    let st = h.get_state(&rt);
    assert!(!st.deadline_cron_active);
    h.check_state(&rt);
}

#[test]
fn sector_expires_and_repays_fee_debt() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    let sectors =
        h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);
    // advance cron to activate power.
    h.advance_and_submit_posts(&mut rt, &sectors);
    let active_power = power_for_sectors(h.sector_size, &sectors);

    let mut st = h.get_state(&rt);
    let initial_pledge = &st.initial_pledge;
    let expiration_raw = sectors[0].expiration;
    assert!(st.deadline_cron_active);

    // setup state to simulate moving forward all the way to expiry
    let (dl_idx, _) = st.find_sector(&rt.policy, &rt.store, sectors[0].sector_number).unwrap();
    let exp_quant_spec = st.quant_spec_for_deadline(&rt.policy, dl_idx);
    let expiration = exp_quant_spec.quantize_up(expiration_raw);
    let remaining_epoch = expiration - st.proving_period_start;
    let remaining_period = remaining_epoch / rt.policy.wpost_proving_period + 1;
    st.proving_period_start += remaining_period * rt.policy.wpost_proving_period;
    st.current_deadline = dl_idx;
    rt.replace_state(&st);

    // Advance to expiration epoch and expect expiration during cron
    rt.set_epoch(expiration);
    let power_delta = active_power.neg();

    // introduce lots of fee debt
    let mut st = h.get_state(&rt);
    let fee_debt = TokenAmount::from(400) * TokenAmount::from(1e18 as u64);
    st.fee_debt = fee_debt;
    rt.replace_state(&st);
    // Miner balance = IP, debt repayment covered by unlocked funds
    rt.set_balance(st.initial_pledge);

    // because we skip forward in state and don't check post, there's no penalty.
    // this is the first time the sector is detected faulty
    h.advance_deadline(
        &mut rt,
        CronConfig {
            no_enrollment: true,
            expired_sectors_power_delta: Some(power_delta),
            expired_sectors_pledge_delta: initial_pledge.neg(),
            repaid_fee_debt: initial_pledge.clone(),
            ..CronConfig::default()
        },
    );
    let st = h.get_state(&rt);
    assert!(!st.deadline_cron_active);
    h.check_state(&rt);
}

#[test]
fn detects_and_penalizes_faults() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    let active_sectors =
        h.commit_and_prove_sectors(&mut rt, 2, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);
    // advance cron to activate power.
    h.advance_and_submit_posts(&mut rt, &active_sectors);
    let active_power = power_for_sectors(h.sector_size, &active_sectors);

    let unproven_sectors =
        h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], false);
    let unproven_power = power_for_sectors(h.sector_size, &unproven_sectors);

    let total_power = &unproven_power + &active_power;
    let all_sectors = [active_sectors.clone(), unproven_sectors].concat();

    // add lots of funds so penalties come from vesting funds
    h.apply_rewards(&mut rt, TokenAmount::from(BIG_REWARDS), TokenAmount::zero());

    let st = h.get_state(&rt);
    let (dl_idx, p_idx) =
        st.find_sector(&rt.policy, &rt.store, active_sectors[0].sector_number).unwrap();

    // advance to next deadline where we expect the first sectors to appear
    let mut dl_info = h.deadline(&rt);
    while dl_info.index != dl_idx {
        dl_info = h.advance_deadline(&mut rt, CronConfig::default());
    }

    // Skip to end of the deadline, cron detects sectors as faulty
    let active_power_delta = active_power.neg();
    h.advance_deadline(
        &mut rt,
        CronConfig { detected_faults_power_delta: Some(active_power_delta), ..Default::default() },
    );

    // expect faulty power to be added to state
    let deadline = h.get_deadline(&rt, dl_idx);
    assert_eq!(total_power, deadline.faulty_power);

    // advance 3 deadlines
    h.advance_deadline(&mut rt, CronConfig::default());
    h.advance_deadline(&mut rt, CronConfig::default());
    dl_info = h.advance_deadline(&mut rt, CronConfig::default());

    h.declare_recoveries(
        &mut rt,
        dl_idx,
        p_idx,
        sector_info_as_bitfield(&all_sectors[1..]),
        TokenAmount::zero(),
    )
    .unwrap();

    // Skip to end of proving period for sectors, cron detects all sectors as faulty
    while dl_info.index != dl_idx {
        dl_info = h.advance_deadline(&mut rt, CronConfig::default());
    }

    // Un-recovered faults (incl failed recovery) are charged as ongoing faults
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

    // recorded faulty power is unchanged
    let deadline = h.get_deadline(&rt, dl_idx);
    assert_eq!(total_power, deadline.faulty_power);

    check_deadline_invariants(
        &rt,
        st.quant_spec_for_deadline(&rt.policy, dl_idx),
        h.sector_size,
        &all_sectors,
        &deadline,
    );
    h.check_state(&rt);
}

#[test]
fn test_cron_run_trigger_faults() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    // add lots of funds so we can pay penalties without going into debt
    h.apply_rewards(&mut rt, TokenAmount::from(BIG_REWARDS), TokenAmount::zero());

    // create enough sectors that one will be in a different partition
    let all_sectors =
        h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);

    // advance cron to activate power.
    h.advance_and_submit_posts(&mut rt, &all_sectors);

    let st = h.get_state(&rt);
    let (dl_idx, _) = st.find_sector(&rt.policy, &rt.store, all_sectors[0].sector_number).unwrap();

    // advance to deadline prior to first
    let mut dl_info = h.deadline(&rt);
    while dl_info.index != dl_idx {
        dl_info = h.advance_deadline(&mut rt, CronConfig::default());
    }

    rt.set_epoch(dl_info.last());

    // run cron and expect all sectors to be detected as faults (no penalty)
    let pwr = power_for_sectors(h.sector_size, &all_sectors);

    // power for sectors is removed
    let power_delta_claim = PowerPair { raw: pwr.raw.neg(), qa: pwr.qa.neg() };

    // expect next cron to be one deadline period after expected cron for this deadline
    let next_cron = dl_info.last() + rt.policy.wpost_challenge_window;

    h.on_deadline_cron(
        &mut rt,
        CronConfig {
            expected_enrollment: next_cron,
            detected_faults_power_delta: Some(power_delta_claim),
            ..CronConfig::default()
        },
    );
    h.check_state(&rt);
}

fn sector_info_as_bitfield(sectors: &[SectorOnChainInfo]) -> BitField {
    let mut bf = BitField::new();
    for sector in sectors {
        bf.set(sector.sector_number);
    }
    bf
}

// check the deadline's invariants, returning all contained sectors, faults,
// recoveries, terminations, and partition/sector assignments.
pub fn check_deadline_invariants(
    rt: &MockRuntime,
    quant_spec: QuantSpec,
    sector_size: SectorSize,
    sectors: &[SectorOnChainInfo],
    deadline: &Deadline,
) -> DeadlineStateSummary {
    let acc = MessageAccumulator::default();
    let summary = check_deadline_state_invariants(
        deadline,
        &rt.store,
        quant_spec,
        sector_size,
        &sectors_as_map(sectors),
        &acc,
    );

    assert!(acc.is_empty(), "{}", acc.messages().join("\n"));

    summary
}
