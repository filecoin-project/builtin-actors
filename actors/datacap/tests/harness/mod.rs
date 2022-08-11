use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;
use fvm_shared::MethodNum;
use lazy_static::lazy_static;

use fil_actor_datacap::testing::check_state_invariants;
use fil_actor_datacap::{Actor as DataCapActor, Method, State};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::*;
use fil_actors_runtime::{SYSTEM_ACTOR_ADDR, VERIFIED_REGISTRY_ACTOR_ADDR};

lazy_static! {
    pub static ref RECEIVER_ADDR: Address = Address::new_id(10);
    pub static ref REGISTRY_ADDR: Address = *VERIFIED_REGISTRY_ACTOR_ADDR;
}

pub fn new_runtime() -> MockRuntime {
    MockRuntime {
        receiver: *RECEIVER_ADDR,
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

#[allow(dead_code)]
pub fn new_harness() -> (Harness, MockRuntime) {
    let mut rt = new_runtime();
    let h = Harness { registry: *REGISTRY_ADDR };
    h.construct_and_verify(&mut rt, &h.registry);
    (h, rt)
}

pub struct Harness {
    pub registry: Address,
}

impl Harness {
    pub fn construct_and_verify(&self, rt: &mut MockRuntime, registry: &Address) {
        rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);
        let ret = rt
            .call::<DataCapActor>(
                Method::Constructor as MethodNum,
                &RawBytes::serialize(registry).unwrap(),
            )
            .unwrap();

        assert_eq!(RawBytes::default(), ret);
        rt.verify();

        let state: State = rt.get_state();
        assert_eq!(self.registry, state.registry);
    }

    pub fn check_state(&self, rt: &MockRuntime) {
        let (_, acc) = check_state_invariants(&rt.get_state(), rt.store());
        acc.assert_empty();
    }
}
