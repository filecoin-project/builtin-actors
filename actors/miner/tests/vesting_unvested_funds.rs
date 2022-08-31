use fil_actor_miner::VestSpec;
use fil_actor_miner::VestingFunds;
use fvm_ipld_encoding::CborStore;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;

mod state_harness;
use state_harness::*;

#[test]
fn unlock_unvested_funds_leaving_bucket_with_non_zero_tokens() {
    let mut h = StateHarness::new(0);
    let vspec = VestSpec { initial_delay: 0, vest_period: 5, step_duration: 1, quantization: 1 };

    let vest_start = 100;
    let vest_sum = TokenAmount::from_atto(100);

    h.add_locked_funds(vest_start, &vest_sum, &vspec).unwrap();

    let amount_unlocked = h.unlock_unvested_funds(vest_start, &TokenAmount::from_atto(39)).unwrap();
    assert_eq!(TokenAmount::from_atto(39), amount_unlocked);

    // no vested funds available to unlock until strictly after first vesting epoch
    assert_eq!(TokenAmount::zero(), h.unlock_vested_funds(vest_start).unwrap());
    assert_eq!(TokenAmount::zero(), h.unlock_vested_funds(vest_start + 1).unwrap());

    // expected to be zero due to unlocking of UNvested funds
    assert_eq!(TokenAmount::zero(), h.unlock_vested_funds(vest_start + 2).unwrap());
    // expected to be partially unlocked already du to unlocking of UNvested funds
    assert_eq!(TokenAmount::from_atto(1), h.unlock_vested_funds(vest_start + 3).unwrap());

    assert_eq!(TokenAmount::from_atto(20), h.unlock_vested_funds(vest_start + 4).unwrap());
    assert_eq!(TokenAmount::from_atto(20), h.unlock_vested_funds(vest_start + 5).unwrap());
    assert_eq!(TokenAmount::from_atto(20), h.unlock_vested_funds(vest_start + 6).unwrap());

    assert_eq!(TokenAmount::zero(), h.unlock_vested_funds(vest_start + 7).unwrap());

    assert!(h.st.locked_funds.is_zero());
    assert!(h.vesting_funds_store_empty())
}

#[test]
fn unlock_unvested_funds_leaving_bucket_with_zero_tokens() {
    let mut h = StateHarness::new(0);
    let vspec = VestSpec { initial_delay: 0, vest_period: 5, step_duration: 1, quantization: 1 };

    let vest_start = 100;
    let vest_sum = TokenAmount::from_atto(100);

    h.add_locked_funds(vest_start, &vest_sum, &vspec).unwrap();

    let amount_unlocked = h.unlock_unvested_funds(vest_start, &TokenAmount::from_atto(40)).unwrap();
    assert_eq!(TokenAmount::from_atto(40), amount_unlocked);

    assert_eq!(TokenAmount::zero(), h.unlock_vested_funds(vest_start).unwrap());
    assert_eq!(TokenAmount::zero(), h.unlock_vested_funds(vest_start + 1).unwrap());

    // expected to be zero due to unlocking of UNvested funds
    assert_eq!(TokenAmount::zero(), h.unlock_vested_funds(vest_start + 2).unwrap());
    assert_eq!(TokenAmount::zero(), h.unlock_vested_funds(vest_start + 3).unwrap());

    assert_eq!(TokenAmount::from_atto(20), h.unlock_vested_funds(vest_start + 4).unwrap());
    assert_eq!(TokenAmount::from_atto(20), h.unlock_vested_funds(vest_start + 5).unwrap());
    assert_eq!(TokenAmount::from_atto(20), h.unlock_vested_funds(vest_start + 6).unwrap());

    assert_eq!(TokenAmount::zero(), h.unlock_vested_funds(vest_start + 7).unwrap());

    assert!(h.st.locked_funds.is_zero());
    assert!(h.vesting_funds_store_empty())
}

#[test]
fn unlock_all_unvested_funds() {
    let mut h = StateHarness::new(0);
    let vspec = VestSpec { initial_delay: 0, vest_period: 5, step_duration: 1, quantization: 1 };

    let vest_start = 10;
    let vest_sum = TokenAmount::from_atto(100);

    h.add_locked_funds(vest_start, &vest_sum, &vspec).unwrap();
    let unvested_funds = h.unlock_unvested_funds(vest_start, &vest_sum).unwrap();
    assert_eq!(vest_sum, unvested_funds);

    assert!(h.st.locked_funds.is_zero());
    assert!(h.vesting_funds_store_empty())
}

#[test]
fn unlock_unvested_funds_value_greater_than_locked_funds() {
    let mut h = StateHarness::new(0);
    let vspec = VestSpec { initial_delay: 0, vest_period: 5, step_duration: 1, quantization: 1 };

    let vest_start = 10;
    let vest_sum = TokenAmount::from_atto(100);
    h.add_locked_funds(vest_start, &vest_sum, &vspec).unwrap();
    let unvested_funds = h.unlock_unvested_funds(vest_start, &TokenAmount::from_atto(200)).unwrap();
    assert_eq!(vest_sum, unvested_funds);

    assert!(h.st.locked_funds.is_zero());
    assert!(h.vesting_funds_store_empty())
}

#[test]
fn unlock_unvested_funds_when_there_are_vested_funds_in_the_table() {
    let mut h = StateHarness::new(0);
    let vspec = VestSpec { initial_delay: 0, vest_period: 50, step_duration: 1, quantization: 1 };

    let vest_start = 10;
    let vest_sum = TokenAmount::from_atto(100);

    // will lock funds from epochs 11 to 60
    h.add_locked_funds(vest_start, &vest_sum, &vspec).unwrap();

    // unlock funds from epochs 30 to 60
    let new_epoch = 30;
    let target = TokenAmount::from_atto(60);
    let remaining = &vest_sum - &target;
    let unvested_funds = h.unlock_unvested_funds(new_epoch, &target).unwrap();
    assert_eq!(target, unvested_funds);

    assert_eq!(remaining, h.st.locked_funds);

    // vesting funds should have all epochs from 11 to 29
    let vesting = h.store.get_cbor::<VestingFunds>(&h.st.vesting_funds).unwrap().unwrap();
    let mut epoch = 11;
    for vf in vesting.funds {
        assert_eq!(epoch, vf.epoch);
        epoch += 1;
        if epoch == 30 {
            break;
        }
    }
}
