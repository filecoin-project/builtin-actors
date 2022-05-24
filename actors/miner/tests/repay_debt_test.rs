mod state_harness;
use fvm_shared::econ::TokenAmount;
use state_harness::*;

use num_traits::Zero;

#[test]
fn repay_debt_in_priority_order() {
    let mut h = StateHarness::new(0);

    let current_balance = TokenAmount::from(300u16);
    let fee = TokenAmount::from(1000);

    h.st.apply_penalty(&fee).unwrap();
    assert_eq!(h.st.fee_debt, fee);

    let (penalty_from_vesting, penalty_from_balance) =
        h.st.repay_partial_debt_in_priority_order(&h.store, 0, &current_balance).unwrap();
    assert_eq!(penalty_from_vesting, TokenAmount::zero());
    assert_eq!(penalty_from_balance, current_balance);

    let expected_debt = -(current_balance - fee);
    assert_eq!(expected_debt, h.st.fee_debt);

    let current_balance = TokenAmount::zero();
    let fee = TokenAmount::from(2050);
    h.st.apply_penalty(&fee).unwrap();

    h.st.repay_partial_debt_in_priority_order(&h.store, 33, &current_balance).unwrap();
    let expected_debt = expected_debt + fee;
    assert_eq!(expected_debt, h.st.fee_debt);
}
