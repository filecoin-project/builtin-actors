use fil_actor_miner::{
    pledge_penalty_for_termination, pledge_penalty_for_termination_lower_bound,
    INITIAL_PLEDGE_FACTOR, TERMINATION_LIFETIME_CAP, TERMINATION_REWARD_FACTOR_DENOM,
    TERMINATION_REWARD_FACTOR_NUM,
};
use fil_actors_runtime::EPOCHS_IN_DAY;
use fvm_shared::bigint::{BigInt, Zero};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
use fvm_shared::smooth::FilterEstimate;

fn epoch_target_reward() -> TokenAmount {
    TokenAmount::from(1_u128 << 50)
}

// 1 64 GiB sector
fn qa_sector_power() -> StoragePower {
    StoragePower::from(1_u128 << 36)
}

// 1 PiB of network power, no estimated changes
fn network_qa_power() -> StoragePower {
    StoragePower::from(1_u128 << 50)
}

// exactly 1 attoFIL per byte of power, no estimated changes
fn reward_estimate() -> FilterEstimate {
    FilterEstimate::new(epoch_target_reward(), BigInt::zero())
}

fn power_estimate() -> FilterEstimate {
    FilterEstimate::new(network_qa_power(), BigInt::zero())
}

fn undeclared_penalty() -> TokenAmount {
    pledge_penalty_for_termination_lower_bound(
        &reward_estimate(),
        &power_estimate(),
        &qa_sector_power(),
    )
}

#[test]
fn when_undeclared_fault_fee_exceeds_expected_reward_returns_undeclared_fault_fee() {
    // small pledge compared to current expected reward means
    let initial_pledge = TokenAmount::from(1 << 10);
    let day_reward = initial_pledge / INITIAL_PLEDGE_FACTOR;
    let twenty_day_reward = &day_reward * INITIAL_PLEDGE_FACTOR;
    let sector_age_in_days = 20;
    let sector_age = sector_age_in_days * EPOCHS_IN_DAY;

    let fee = pledge_penalty_for_termination(
        &day_reward,
        sector_age,
        &twenty_day_reward,
        &power_estimate(),
        &qa_sector_power(),
        &reward_estimate(),
        &TokenAmount::zero(),
        0,
    );

    assert_eq!(undeclared_penalty(), fee);
}

#[test]
fn when_expected_reward_exceeds_undeclared_fault_fee_returns_expected_reward() {
    // initialPledge equal to undeclaredPenalty guarantees expected reward is greater
    let initial_pledge = undeclared_penalty();
    let day_reward = &initial_pledge / INITIAL_PLEDGE_FACTOR;
    let twenty_day_reward = &day_reward * INITIAL_PLEDGE_FACTOR;
    let sector_age_in_days = 20;
    let sector_age = sector_age_in_days * EPOCHS_IN_DAY;

    let fee = pledge_penalty_for_termination(
        &day_reward,
        sector_age,
        &twenty_day_reward,
        &power_estimate(),
        &qa_sector_power(),
        &reward_estimate(),
        &TokenAmount::zero(),
        0,
    );

    // expect fee to be pledge + br * age * factor where br = pledge/initialPledgeFactor
    let expected_fee = &initial_pledge
        + (&day_reward * sector_age_in_days * &*TERMINATION_REWARD_FACTOR_NUM)
            / &*TERMINATION_REWARD_FACTOR_DENOM;
    assert_eq!(expected_fee, fee);
}

#[test]
fn sector_age_is_capped() {
    let initial_pledge = undeclared_penalty();
    let day_reward = &initial_pledge / INITIAL_PLEDGE_FACTOR;
    let twenty_day_reward = &day_reward * INITIAL_PLEDGE_FACTOR;
    let sector_age_in_days = 500;
    let sector_age = sector_age_in_days * EPOCHS_IN_DAY;

    let fee = pledge_penalty_for_termination(
        &day_reward,
        sector_age,
        &twenty_day_reward,
        &power_estimate(),
        &qa_sector_power(),
        &reward_estimate(),
        &TokenAmount::zero(),
        0,
    );

    // expect fee to be pledge * br * age-cap * factor where br = pledge/initialPledgeFactor
    let expected_fee = &initial_pledge
        + (&day_reward * TERMINATION_LIFETIME_CAP * &*TERMINATION_REWARD_FACTOR_NUM)
            / &*TERMINATION_REWARD_FACTOR_DENOM;
    assert_eq!(expected_fee, fee);
}

#[test]
fn fee_for_replacement_eq_fee_for_original_sector_when_power_br_are_unchanged() {
    // initialPledge equal to undeclaredPenalty guarantees expected reward is greater
    let initial_pledge = undeclared_penalty();
    let day_reward = &initial_pledge / INITIAL_PLEDGE_FACTOR;
    let twenty_day_reward = &day_reward * INITIAL_PLEDGE_FACTOR;
    let sector_age = 20 * EPOCHS_IN_DAY;
    let replacement_age = 2 * EPOCHS_IN_DAY;

    // use low power, so we don't test SP=SP
    let power = BigInt::from(1);

    // fee for old sector if had terminated when it was replaced
    let unreplaced_fee = pledge_penalty_for_termination(
        &day_reward,
        sector_age,
        &twenty_day_reward,
        &power_estimate(),
        &power,
        &reward_estimate(),
        &TokenAmount::zero(),
        0,
    );

    // actual fee including replacement parameters
    let actual_fee = pledge_penalty_for_termination(
        &day_reward,
        replacement_age,
        &twenty_day_reward,
        &power_estimate(),
        &power,
        &reward_estimate(),
        &day_reward,
        sector_age - replacement_age,
    );

    assert_eq!(unreplaced_fee, actual_fee);
}

#[test]
fn fee_for_replacement_eq_fee_for_same_sector_without_replacement_after_lifetime_cap() {
    // initialPledge equal to undeclaredPenalty guarantees expected reward is greater
    let initial_pledge = undeclared_penalty();
    let day_reward = &initial_pledge / INITIAL_PLEDGE_FACTOR;
    let twenty_day_reward = &day_reward * INITIAL_PLEDGE_FACTOR;
    let sector_age = 20 * EPOCHS_IN_DAY;
    let replacement_age = (TERMINATION_LIFETIME_CAP + 1) * EPOCHS_IN_DAY;

    // use low power, so we don't test SP=SP
    let power = BigInt::from(1);

    // fee for new sector with no replacement
    let noreplace = pledge_penalty_for_termination(
        &day_reward,
        replacement_age,
        &twenty_day_reward,
        &power_estimate(),
        &power,
        &reward_estimate(),
        &TokenAmount::zero(),
        0,
    );

    // actual fee including replacement parameters
    let with_replace = pledge_penalty_for_termination(
        &day_reward,
        replacement_age,
        &twenty_day_reward,
        &power_estimate(),
        &power,
        &reward_estimate(),
        &day_reward,
        sector_age,
    );

    assert_eq!(noreplace, with_replace);
}

#[test]
fn charges_for_replaced_sector_at_replaced_sector_day_rate() {
    // initialPledge equal to undeclaredPenalty guarantees expected reward is greater
    let initial_pledge = undeclared_penalty();
    let day_reward = &initial_pledge / INITIAL_PLEDGE_FACTOR;
    let old_day_reward = 2 * &day_reward;
    let twenty_day_reward = &day_reward * INITIAL_PLEDGE_FACTOR;
    let old_sector_age_in_days = 20;
    let old_sector_age = old_sector_age_in_days * EPOCHS_IN_DAY;
    let replacement_age_in_days = 15;
    let replacement_age = replacement_age_in_days * EPOCHS_IN_DAY;

    // use low power, so termination fee exceeds SP
    let power = BigInt::from(1);

    let old_penalty = (&old_day_reward * old_sector_age_in_days * &*TERMINATION_REWARD_FACTOR_NUM)
        / &*TERMINATION_REWARD_FACTOR_DENOM;
    let new_penalty = (&day_reward * replacement_age_in_days * &*TERMINATION_REWARD_FACTOR_NUM)
        / &*TERMINATION_REWARD_FACTOR_DENOM;
    let expected_fee = &twenty_day_reward + old_penalty + new_penalty;

    let fee = pledge_penalty_for_termination(
        &day_reward,
        replacement_age,
        &twenty_day_reward,
        &power_estimate(),
        &power,
        &reward_estimate(),
        &old_day_reward,
        old_sector_age,
    );

    assert_eq!(expected_fee, fee);
}
