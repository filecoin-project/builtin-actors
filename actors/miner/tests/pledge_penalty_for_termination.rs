use fil_actor_miner::{
    pledge_penalty_for_termination, TERMINATION_LIFETIME_CAP,
    TERM_FEE_MAX_FAULT_FEE_MULTIPLE_DENOM, TERM_FEE_MAX_FAULT_FEE_MULTIPLE_NUM,
    TERM_FEE_MIN_PLEDGE_MULTIPLE_DENOM, TERM_FEE_MIN_PLEDGE_MULTIPLE_NUM,
    TERM_FEE_PLEDGE_MULTIPLE_DENOM, TERM_FEE_PLEDGE_MULTIPLE_NUM,
};
use fil_actors_runtime::EPOCHS_IN_DAY;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;

mod util;

// Not considering fault fees, for a sector where its age >= `TERMINATION_LIFETIME_CAP`, termination fee should equal `TERM_FEE_PLEDGE_MULTIPLE_PERCENTAGE * initial pledge`
#[test]
fn when_sector_age_exceeds_cap_returns_percentage_of_initial_pledge() {
    let sector_age_in_days = TERMINATION_LIFETIME_CAP + 1;
    let sector_age = sector_age_in_days * EPOCHS_IN_DAY;

    let initial_pledge = TokenAmount::from_atto(1 << 10);
    let fault_fee = TokenAmount::zero();
    let fee = pledge_penalty_for_termination(&initial_pledge, sector_age, &fault_fee);
    assert_eq!(
        (TERM_FEE_PLEDGE_MULTIPLE_NUM * initial_pledge).div_floor(TERM_FEE_PLEDGE_MULTIPLE_DENOM),
        fee
    );
}

// Not considering fault fees, for a sector where its age < `TERMINATION_LIFETIME_CAP`, termination fee should equal `TERM_FEE_PLEDGE_MULTIPLE_PERCENTAGE * of initial pledge * sector age in days / TERMINATION_LIFETIME_CAP`
#[test]
fn when_sector_age_below_cap_returns_percentage_of_initial_pledge_percentage() {
    let sector_age_in_days = TERMINATION_LIFETIME_CAP / 2;
    let sector_age = sector_age_in_days * EPOCHS_IN_DAY;

    let initial_pledge = TokenAmount::from_atto(1 << 10);
    let fault_fee = TokenAmount::zero();
    let fee = pledge_penalty_for_termination(&initial_pledge, sector_age, &fault_fee);

    assert_eq!(
        ((TERM_FEE_PLEDGE_MULTIPLE_NUM * initial_pledge).div_floor(TERM_FEE_PLEDGE_MULTIPLE_DENOM)
            * sector_age_in_days)
            .div_floor(TERMINATION_LIFETIME_CAP),
        fee
    );
}

// Considering fault fees, for a sector with a termination fee that is less than the associated sector's fault fee, termination fee should equal `TERM_FEE_MAX_FAULT_FEE_MULTIPLE * fault fee`
#[test]
fn when_termination_fee_less_than_fault_fee_returns_multiple_of_fault_fee() {
    let sector_age_in_days = TERMINATION_LIFETIME_CAP + 1;
    let sector_age = sector_age_in_days * EPOCHS_IN_DAY;

    let initial_pledge = TokenAmount::from_atto(1 << 10);
    let fault_fee = TokenAmount::from_atto(1 << 10);
    let fee = pledge_penalty_for_termination(&initial_pledge, sector_age, &fault_fee);

    assert_eq!(
        (fault_fee * TERM_FEE_MAX_FAULT_FEE_MULTIPLE_NUM)
            .div_floor(TERM_FEE_MAX_FAULT_FEE_MULTIPLE_DENOM),
        fee
    );
}

// Given all test cases above, if the termination fee computed is less than `MIN_TERMINATION_FEE * initial pledge`, termination fee should equal `MIN_TERMINATION_FEE * initial pledge
#[test]
fn when_termination_fee_less_than_minimum_returns_minimum() {
    let sector_age = 0;

    let initial_pledge = TokenAmount::from_atto(1 << 10);
    let fault_fee = TokenAmount::zero();
    let fee = pledge_penalty_for_termination(&initial_pledge, sector_age, &fault_fee);

    assert_eq!(
        (initial_pledge * TERM_FEE_MIN_PLEDGE_MULTIPLE_NUM)
            .div_floor(TERM_FEE_MIN_PLEDGE_MULTIPLE_DENOM),
        fee
    );
}
