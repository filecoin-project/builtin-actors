use fvm_shared::address::Address;
use fvm_shared::encoding::{ RawBytes};
use fvm_shared::MethodNum;
use lazy_static::lazy_static;

use hierarchical_sca::{Method};
use fil_actors_runtime::test_utils::{
    MockRuntime, 
    SYSTEM_ACTOR_CODE_ID,
};
use fil_actors_runtime::{
    STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};

use crate::SCAActor;

lazy_static! {
    pub static ref OWNER: Address = Address::new_id(101);
    pub static ref MINER: Address = Address::new_id(201);
    pub static ref ACTOR: Address = Address::new_actor("actor".as_bytes());
}

pub fn new_runtime() -> MockRuntime {
    MockRuntime {
        receiver: *STORAGE_POWER_ACTOR_ADDR,
        caller: *SYSTEM_ACTOR_ADDR,
        caller_type: *SYSTEM_ACTOR_CODE_ID,
        ..Default::default()
    }
}

pub fn new_harness() -> Harness {
    Harness {
    }
}

pub fn setup() -> (Harness, MockRuntime) {
    let mut rt = new_runtime();
    let h = new_harness();
    h.construct(&mut rt);
    (h, rt)
}

#[allow(dead_code)]
pub struct Harness {
}

impl Harness {
    pub fn construct(&self, rt: &mut MockRuntime) {
        rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);
        rt.call::<SCAActor>(Method::Constructor as MethodNum, &RawBytes::default()).unwrap();
        rt.verify()
    }

    pub fn construct_and_verify(&self, rt: &mut MockRuntime) {
        self.construct(rt);
    }

}
