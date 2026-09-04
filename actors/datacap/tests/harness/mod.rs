use std::cell::RefCell;

use fvm_shared::MethodNum;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;

use fil_actor_datacap::testing::check_state_invariants;
use fil_actor_datacap::{Actor as DataCapActor, Method, State};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{
    DATACAP_TOKEN_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR,
};
use fvm_ipld_encoding::ipld_block::IpldBlock;

pub fn new_runtime() -> MockRuntime {
    MockRuntime {
        receiver: DATACAP_TOKEN_ACTOR_ADDR,
        caller: RefCell::new(SYSTEM_ACTOR_ADDR),
        caller_type: RefCell::new(*SYSTEM_ACTOR_CODE_ID),
        ..Default::default()
    }
}

#[allow(dead_code)]
pub fn new_harness() -> (Harness, MockRuntime) {
    let rt = new_runtime();
    let h = Harness { governor: VERIFIED_REGISTRY_ACTOR_ADDR };
    h.construct_and_verify(&rt, &h.governor);
    (h, rt)
}

pub struct Harness {
    pub governor: Address,
}

impl Harness {
    pub fn construct_and_verify(&self, rt: &MockRuntime, registry: &Address) {
        rt.set_caller(*SYSTEM_ACTOR_CODE_ID, SYSTEM_ACTOR_ADDR);
        rt.expect_validate_caller_addr(vec![SYSTEM_ACTOR_ADDR]);
        let ret = rt
            .call::<DataCapActor>(
                Method::Constructor as MethodNum,
                IpldBlock::serialize_cbor(registry).unwrap(),
            )
            .unwrap();

        assert!(ret.is_none());
        rt.verify();

        let state: State = rt.get_state();
        assert_eq!(self.governor, state.governor);
    }

    /// Sets a balance directly in state, bypassing the (now deprecated) Mint method.
    /// FIP-0118: used for test fixture setup since Mint always returns forbidden.
    pub fn mint_directly(&self, rt: &MockRuntime, to: &Address, amount: &TokenAmount) {
        let mut st: State = rt.get_state();
        st.token.change_balance_by(&rt.store(), to.id().unwrap(), amount).unwrap();
        st.token.change_supply_by(amount).unwrap();
        rt.replace_state(&st);
    }

    /// Sets an allowance directly in state. FIP-0118: used for test fixture setup since
    /// Mint no longer grants operator allowances as a side effect.
    pub fn allow_directly(
        &self,
        rt: &MockRuntime,
        owner: &Address,
        operator: &Address,
        amount: &TokenAmount,
    ) {
        let mut st: State = rt.get_state();
        st.token
            .set_allowance(&rt.store(), owner.id().unwrap(), operator.id().unwrap(), amount)
            .unwrap();
        rt.replace_state(&st);
    }

    // Reads a balance from state directly.
    pub fn get_balance(&self, rt: &MockRuntime, address: &Address) -> TokenAmount {
        rt.expect_validate_caller_any();
        let ret = rt
            .call::<DataCapActor>(
                Method::BalanceExported as MethodNum,
                IpldBlock::serialize_cbor(&address).unwrap(),
            )
            .unwrap()
            .unwrap()
            .deserialize()
            .unwrap();
        rt.verify();
        ret
    }

    pub fn check_state(&self, rt: &MockRuntime) {
        let (_, acc) = check_state_invariants(&rt.get_state(), rt.store());
        acc.assert_empty();
    }
}
