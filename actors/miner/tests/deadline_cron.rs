use fil_actor_miner::testing::{DeadlineStateSummary, check_deadline_state_invariants};
use fil_actor_miner::{
    Deadline, PowerPair, QuantSpec, REWARD_VESTING_SPEC, SectorOnChainInfo, daily_fee_for_sectors,
    pledge_penalty_for_continued_fault, power_for_sectors,
};
use fil_actors_runtime::runtime::RuntimePolicy;
use fil_actors_runtime::test_utils::MockRuntime;
use fil_actors_runtime::{EPOCHS_IN_DAY, EPOCHS_IN_HOUR, MessageAccumulator};
use fvm_ipld_bitfield::BitField;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::SectorSize;
use std::ops::Neg;

mod util;
use crate::util::*;

// an expiration ~10 days greater than effective min expiration taking into account 30 days max
// between pre and prove commit
const DEFAULT_SECTOR_EXPIRATION: ChainEpoch = 220;

const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn cron_on_inactive_state() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&rt);

    let st = h.get_state(&rt);
    assert_eq!(PERIOD_OFFSET - rt.policy.wpost_proving_period, st.proving_period_start);
    assert!(!st.continue_deadline_cron());

    // cron does nothing and does not enroll another cron
    let deadline = h.deadline(&rt);
    rt.set_epoch(deadline.last());
    h.on_deadline_cron(&rt, CronConfig { no_enrollment: true, ..CronConfig::default() });

    h.check_state(&rt);
}

#[test]
fn test_vesting_on_cron() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&rt);

    // onboard some sectors and get power
    let active_sectors =
        h.commit_and_prove_sectors(&rt, 2, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);
    // advance cron to activate power.
    h.advance_and_submit_posts(&rt, &active_sectors);

    // --- ADD REWARDS FOR VESTING
    let apply_reward_fn = |target_epoch: ChainEpoch| {
        rt.set_epoch(target_epoch);
        h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());
    };

    let current_epoch = *rt.epoch.borrow();
    apply_reward_fn(current_epoch);

    // MOVE EPOCH BY HALF A DAY AND ADD ONE MORE REWARD SCHEDULE
    let current_epoch = current_epoch + (EPOCHS_IN_DAY / 2) + 100;
    apply_reward_fn(current_epoch);

    // --- ASSERT FUNDS TO BE VESTED
    let st = h.get_state(&rt);
    let vesting_funds = st.vesting_funds.load(&rt.store).unwrap();
    assert_eq!(360, vesting_funds.len());

    let q = QuantSpec { unit: REWARD_VESTING_SPEC.quantization, offset: st.proving_period_start };

    let assert_locked_fn = |new_epoch: ChainEpoch, should_vest: bool| {
        let st = h.get_state(&rt);
        rt.set_epoch(new_epoch);
        let new_deadline_info = st.deadline_info(rt.policy(), new_epoch + 1);
        let vesting_now = immediately_vesting_funds(&rt, &st);
        let old_locked = st.locked_funds;
        h.on_deadline_cron(
            &rt,
            CronConfig {
                pledge_delta: -vesting_now.clone(),
                expected_enrollment: new_deadline_info.last(),
                ..CronConfig::default()
            },
        );
        let new_locked = h.get_state(&rt).locked_funds;
        if should_vest {
            assert!(vesting_now.is_positive());
            assert_ne!(old_locked, new_locked);
        } else {
            assert!(vesting_now.is_zero());
            assert_eq!(old_locked, new_locked);
        }
    };

    // move current epoch by a day so funds get vested. +1 because rewards are vested at the end of
    // an epoch.
    let new_epoch = q.quantize_up(current_epoch + 12 * EPOCHS_IN_HOUR) + 1;
    assert_locked_fn(new_epoch, true);

    // no funds get vested if epoch moves by <12 hours
    let new_epoch = new_epoch + (12 * EPOCHS_IN_HOUR) - 100;
    assert_locked_fn(new_epoch, false);

    // funds get vested again if epoch is quantised
    let new_epoch = q.quantize_up(new_epoch) + 1;
    assert_locked_fn(new_epoch, true);

    h.check_state(&rt);
}

#[test]
fn sector_expires() {
    let mut h = ActorHarness::new(PERIOD_OFFSET);
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&rt);

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);
    // advance cron to activate power.
    h.advance_and_submit_posts(&rt, &sectors);
    let active_power = power_for_sectors(h.sector_size, &sectors);

    let mut st = h.get_state(&rt);
    let initial_pledge = &st.initial_pledge;
    let expiration_raw = sectors[0].expiration;
    assert!(st.deadline_cron_active);

    // setup state to simulate moving forward all the way to expiry
    let (dl_idx, _) = st.find_sector(&rt.store, sectors[0].sector_number).unwrap();
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
        &rt,
        CronConfig {
            no_enrollment: true,
            power_delta: Some(power_delta),
            pledge_delta: initial_pledge.neg(),
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
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&rt);

    let sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);
    // advance cron to activate power.
    h.advance_and_submit_posts(&rt, &sectors);
    let active_power = power_for_sectors(h.sector_size, &sectors);

    let mut st = h.get_state(&rt);
    let initial_pledge = &st.initial_pledge;
    let expiration_raw = sectors[0].expiration;
    assert!(st.deadline_cron_active);

    // setup state to simulate moving forward all the way to expiry
    let (dl_idx, _) = st.find_sector(&rt.store, sectors[0].sector_number).unwrap();
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
    let fee_debt = TokenAmount::from_whole(400);
    st.fee_debt = fee_debt;
    rt.replace_state(&st);
    // Miner balance = IP, debt repayment covered by unlocked funds
    rt.set_balance(st.initial_pledge);

    // because we skip forward in state and don't check post, there's no penalty.
    // this is the first time the sector is detected faulty
    h.advance_deadline(
        &rt,
        CronConfig {
            no_enrollment: true,
            power_delta: Some(power_delta),
            burnt_funds: initial_pledge.clone(),
            pledge_delta: initial_pledge.neg(),
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
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&rt);

    let active_sectors =
        h.commit_and_prove_sectors(&rt, 2, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);
    // advance cron to activate power.
    h.advance_and_submit_posts(&rt, &active_sectors);
    let active_power = power_for_sectors(h.sector_size, &active_sectors);

    let unproven_sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], false);
    let unproven_power = power_for_sectors(h.sector_size, &unproven_sectors);

    let total_power = &unproven_power + &active_power;
    let all_sectors = [active_sectors.clone(), unproven_sectors].concat();

    // add lots of funds so penalties come from vesting funds
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());

    let st = h.get_state(&rt);
    let (dl_idx, p_idx) = st.find_sector(&rt.store, active_sectors[0].sector_number).unwrap();

    // advance to next deadline where we expect the first sectors to appear
    let mut dl_info = h.deadline(&rt);
    while dl_info.index != dl_idx {
        dl_info = h.advance_deadline(&rt, CronConfig::default());
    }

    // Skip to end of the deadline, cron detects sectors as faulty
    let mut fee_payable = daily_fee_for_sectors(&all_sectors);
    let active_power_delta = active_power.neg();
    h.advance_deadline(
        &rt,
        CronConfig {
            burnt_funds: fee_payable.clone(),
            pledge_delta: -fee_payable.clone(),
            power_delta: Some(active_power_delta),
            ..Default::default()
        },
    );

    // expect faulty power to be added to state
    let deadline = h.get_deadline(&rt, dl_idx);
    assert_eq!(total_power, deadline.faulty_power);

    // advance 3 deadlines
    h.advance_deadline(&rt, CronConfig::default());
    h.advance_deadline(&rt, CronConfig::default());
    dl_info = h.advance_deadline(&rt, CronConfig::default());

    h.declare_recoveries(
        &rt,
        dl_idx,
        p_idx,
        sector_info_as_bitfield(&all_sectors[1..]),
        TokenAmount::zero(),
    )
    .unwrap();

    // Skip to end of proving period for sectors, cron detects all sectors as faulty
    while dl_info.index != dl_idx {
        dl_info = h.advance_deadline(&rt, CronConfig::default());
    }

    // Un-recovered faults (incl failed recovery) are charged as ongoing faults
    let ongoing_pwr = power_for_sectors(h.sector_size, &all_sectors);
    let ongoing_penalty = pledge_penalty_for_continued_fault(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &ongoing_pwr.qa,
    );
    fee_payable += ongoing_penalty;

    h.advance_deadline(
        &rt,
        CronConfig {
            burnt_funds: fee_payable.clone(),
            pledge_delta: -fee_payable,
            ..Default::default()
        },
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
    let rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&rt);

    // add lots of funds so we can pay penalties without going into debt
    h.apply_rewards(&rt, BIG_REWARDS.clone(), TokenAmount::zero());

    // create enough sectors that one will be in a different partition
    let all_sectors =
        h.commit_and_prove_sectors(&rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);

    // advance cron to activate power.
    h.advance_and_submit_posts(&rt, &all_sectors);

    let st = h.get_state(&rt);
    let (dl_idx, _) = st.find_sector(&rt.store, all_sectors[0].sector_number).unwrap();

    // advance to deadline prior to first
    let mut dl_info = h.deadline(&rt);
    while dl_info.index != dl_idx {
        dl_info = h.advance_deadline(&rt, CronConfig::default());
    }

    rt.set_epoch(dl_info.last());

    // run cron and expect all sectors to be detected as faults (no penalty)
    let pwr = power_for_sectors(h.sector_size, &all_sectors);
    let daily_fee = daily_fee_for_sectors(&all_sectors);
    // power for sectors is removed
    let power_delta_claim = PowerPair { raw: pwr.raw.neg(), qa: pwr.qa.neg() };

    // expect next cron to be one deadline period after expected cron for this deadline
    let next_cron = dl_info.last() + rt.policy.wpost_challenge_window;

    h.on_deadline_cron(
        &rt,
        CronConfig {
            burnt_funds: daily_fee.clone(),
            pledge_delta: daily_fee.neg(),
            power_delta: Some(power_delta_claim),
            expected_enrollment: next_cron,
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
