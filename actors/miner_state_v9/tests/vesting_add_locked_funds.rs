use fil_actor_miner_state_v9::VestSpec;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;

mod state_harness;
use state_harness::*;

#[test]
fn locked_funds_increases_with_sequential_calls() {
    let mut h = StateHarness::new(0);
    let vspec = VestSpec { initial_delay: 0, vest_period: 1, step_duration: 1, quantization: 1 };

    let vest_start = 10;
    let vest_sum = TokenAmount::from_atto(100);

    h.add_locked_funds(vest_start, &vest_sum, &vspec).unwrap();
    assert_eq!(vest_sum, h.st.locked_funds);

    h.add_locked_funds(vest_start, &vest_sum, &vspec).unwrap();
    assert_eq!(vest_sum * 2, h.st.locked_funds);
}

#[test]
fn vests_when_quantize_step_duration_and_vesting_period_are_coprime() {
    let mut h = StateHarness::new(0);
    let vspec = VestSpec { initial_delay: 0, vest_period: 27, step_duration: 5, quantization: 7 };

    let vest_start = 10;
    let vest_sum = TokenAmount::from_atto(100);
    h.add_locked_funds(vest_start, &vest_sum, &vspec).unwrap();
    assert_eq!(vest_sum, h.st.locked_funds);

    let mut total_vested = TokenAmount::zero();
    for e in vest_start..=43 {
        let amount_vested = h.unlock_vested_funds(e).unwrap();
        match e {
            22 => {
                assert_eq!(TokenAmount::from_atto(40), amount_vested);
            }
            29 => {
                assert_eq!(TokenAmount::from_atto(26), amount_vested);
            }
            36 => {
                assert_eq!(TokenAmount::from_atto(26), amount_vested);
            }
            43 => {
                assert_eq!(TokenAmount::from_atto(8), amount_vested);
            }
            _ => {
                assert_eq!(TokenAmount::zero(), amount_vested);
            }
        }
        total_vested += amount_vested;
    }
    assert_eq!(vest_sum, total_vested);
    assert!(h.st.locked_funds.is_zero());
    assert!(h.vesting_funds_store_empty())
}
