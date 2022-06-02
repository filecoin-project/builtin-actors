use fil_actor_miner::locked_reward_from_reward;
use fil_actor_miner::ApplyRewardParams;
use fil_actor_miner::REWARD_VESTING_SPEC;
use fil_actor_miner::{Actor, Method};
use fil_actor_power::Method as PowerMethod;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::runtime::RuntimePolicy;
use fil_actors_runtime::test_utils::REWARD_ACTOR_CODE_ID;
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fil_actors_runtime::REWARD_ACTOR_ADDR;
use fil_actors_runtime::STORAGE_POWER_ACTOR_ADDR;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::bigint::bigint_ser::BigIntSer;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::{ChainEpoch, QuantSpec};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;

mod util;
use num_traits::Signed;
use util::*;

const BIG_BALANCE: u128 = 1_000_000_000_000_000_000_000_000u128;
const PERIOD_OFFSET: ChainEpoch = 1808;

#[test]
fn funds_are_locked() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    let rwd = TokenAmount::from(1_000_000);
    h.apply_rewards(&mut rt, rwd, TokenAmount::zero());

    let expected = TokenAmount::from(750_000);
    assert_eq!(expected, h.get_locked_funds(&rt));
}

#[test]
fn funds_vest() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);
    let st = h.get_state(&rt);

    let vesting_funds = st.load_vesting_funds(&rt.store).unwrap();

    // Nothing vesting to start
    assert!(vesting_funds.funds.is_empty());
    assert!(st.locked_funds.is_zero());

    // Lock some funds with AddLockedFund
    let amt = TokenAmount::from(600_000);
    h.apply_rewards(&mut rt, amt.clone(), TokenAmount::zero());
    let st = h.get_state(&rt);
    let vesting_funds = st.load_vesting_funds(&rt.store).unwrap();

    assert_eq!(180, vesting_funds.funds.len());

    // Vested FIL pays out on epochs with expected offset
    let quant_spec = QuantSpec { unit: REWARD_VESTING_SPEC.quantization, offset: PERIOD_OFFSET };

    let curr_epoch = rt.epoch;
    for (i, vf) in vesting_funds.funds.iter().enumerate() {
        let step =
            REWARD_VESTING_SPEC.initial_delay + (i as i64 + 1) * REWARD_VESTING_SPEC.step_duration;
        let expected_epoch = quant_spec.quantize_up(curr_epoch + step);
        assert_eq!(expected_epoch, vf.epoch);
    }

    let expected_offset = PERIOD_OFFSET % REWARD_VESTING_SPEC.quantization;
    for vf in vesting_funds.funds.iter() {
        assert_eq!(expected_offset, vf.epoch % REWARD_VESTING_SPEC.quantization);
    }

    let st = h.get_state(&rt);
    let (locked_amt, _) = locked_reward_from_reward(amt);
    assert_eq!(locked_amt, st.locked_funds);
    // technically applying rewards without first activating cron is an impossible state but convenient for testing
    let (_, acc) = check_state_invariants(rt.policy(), st, rt.store(), &rt.get_balance());
    assert_eq!(1, acc.len());
    assert!(acc.messages().first().unwrap().contains("DeadlineCronActive == false"));
}

#[test]
fn penalty_is_burnt() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);

    let rwd = TokenAmount::from(600_000);
    let penalty = TokenAmount::from(300_000);
    rt.add_balance(rwd.clone());
    h.apply_rewards(&mut rt, rwd.clone(), penalty.clone());

    let (mut expected_lock_amt, _) = locked_reward_from_reward(rwd);
    expected_lock_amt -= penalty;
    assert_eq!(expected_lock_amt, h.get_locked_funds(&rt));
    // technically applying rewards without first activating cron is an impossible state but convenient for testing
    let (_, acc) =
        check_state_invariants(rt.policy(), h.get_state(&rt), rt.store(), &rt.get_balance());
    assert_eq!(1, acc.len());
    assert!(acc.messages().first().unwrap().contains("DeadlineCronActive == false"));
}

#[test]
fn penalty_is_partially_burnt_and_stored_as_fee_debt() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);
    let st = h.get_state(&rt);
    assert!(st.fee_debt.is_zero());

    let amt = rt.get_balance();
    let penalty = &amt * 3;
    let reward = amt.clone();

    // manually update actor balance to include the added funds on reward message
    let new_balance = &reward + &amt;
    rt.set_balance(new_balance);

    rt.set_caller(*REWARD_ACTOR_CODE_ID, *REWARD_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![*REWARD_ACTOR_ADDR]);

    // pledge change is new reward - reward taken for fee debt
    // zero here since all reward goes to debt
    // so do not expect pledge update

    // burn initial balance + reward = 2*amt
    let expect_burnt = 2 * &amt;
    rt.expect_send(
        *BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        RawBytes::default(),
        expect_burnt,
        RawBytes::default(),
        ExitCode::OK,
    );

    let params = ApplyRewardParams { reward, penalty };
    rt.call::<Actor>(Method::ApplyRewards as u64, &RawBytes::serialize(params).unwrap()).unwrap();
    rt.verify();

    let st = h.get_state(&rt);
    // fee debt =  penalty - reward - initial balance = 3*amt - 2*amt = amt
    assert_eq!(amt, st.fee_debt);
    // technically applying rewards without first activating cron is an impossible state but convenient for testing
    h.check_state(&rt);
}

// The system should not reach this state since fee debt removes mining eligibility
// But if invariants are violated this should work.
#[test]
fn rewards_pay_back_fee_debt() {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(TokenAmount::from(BIG_BALANCE));
    h.construct_and_verify(&mut rt);
    let mut st = h.get_state(&rt);

    assert!(st.locked_funds.is_zero());

    let amt = rt.get_balance();
    let available_before = st.get_available_balance(&amt).unwrap();
    assert!(available_before.is_positive());
    let init_fee_debt: TokenAmount = 2 * &amt; // FeeDebt twice total balance
    st.fee_debt = init_fee_debt.clone();
    let available_after = st.get_available_balance(&amt).unwrap();
    assert!(available_after.is_negative());

    rt.replace_state(&st);

    let reward: TokenAmount = 3 * &amt;
    let penalty = TokenAmount::zero();
    // manually update actor balance to include the added funds from outside
    let new_balance = &amt + &reward;
    rt.set_balance(new_balance.clone());

    // pledge change is new reward - reward taken for fee debt
    // 3*LockedRewardFactor*amt - 2*amt = remainingLocked
    let (locked_reward, _) = locked_reward_from_reward(reward.clone());
    let remaining_locked = locked_reward - &st.fee_debt; // note that this would be clamped at 0 if difference above is < 0
    assert!(remaining_locked.is_positive());
    let pledge_delta = remaining_locked.clone();
    rt.set_caller(*REWARD_ACTOR_CODE_ID, *REWARD_ACTOR_ADDR);
    rt.expect_validate_caller_addr(vec![*REWARD_ACTOR_ADDR]);
    // expect pledge update
    rt.expect_send(
        *STORAGE_POWER_ACTOR_ADDR,
        PowerMethod::UpdatePledgeTotal as u64,
        RawBytes::serialize(BigIntSer(&pledge_delta)).unwrap(),
        TokenAmount::from(0u8),
        RawBytes::default(),
        ExitCode::OK,
    );

    let expect_burnt = st.fee_debt;
    rt.expect_send(
        *BURNT_FUNDS_ACTOR_ADDR,
        METHOD_SEND,
        RawBytes::default(),
        expect_burnt.clone(),
        RawBytes::default(),
        ExitCode::OK,
    );

    let params = ApplyRewardParams { reward: reward.clone(), penalty };
    rt.call::<Actor>(Method::ApplyRewards as u64, &RawBytes::serialize(params).unwrap()).unwrap();
    rt.verify();

    // Set balance to deduct fee
    let final_balance = &new_balance - &expect_burnt;

    let st = h.get_state(&rt);
    // balance funds used to pay off fee debt
    // available balance should be 2
    let available_balance = st.get_available_balance(&final_balance).unwrap();
    assert_eq!(available_before + reward - init_fee_debt - &remaining_locked, available_balance);
    assert!(!st.fee_debt.is_positive());
    // remaining funds locked in vesting table
    assert_eq!(remaining_locked, st.locked_funds);
    // technically applying rewards without first activating cron is an impossible state but convenient for testing
    let (_, acc) = check_state_invariants(rt.policy(), st, rt.store(), &rt.get_balance());
    assert_eq!(1, acc.len());
    assert!(acc.messages().first().unwrap().contains("DeadlineCronActive == false"));
}
