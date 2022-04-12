use cid::Cid;
// use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::{RawBytes};
// use fvm_ipld_hamt::BytesKey;
// use fvm_ipld_hamt::Error;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser::BigIntDe;
// use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
// use fvm_shared::error::ExitCode;
use fvm_shared::MethodNum;
use lazy_static::lazy_static;
// use num_traits::Zero;

// use serde::Serialize;

use fil_actors_runtime::builtin::HAMT_BIT_WIDTH;
// use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::test_utils::{
    MockRuntime, SYSTEM_ACTOR_CODE_ID,
};
use fil_actors_runtime::{
    make_map_with_root_and_bitwidth,  STORAGE_POWER_ACTOR_ADDR,
    SYSTEM_ACTOR_ADDR,
};

use hierarchical_sca::{Method, State, ConstructorParams};

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
    pub const nn: String = String::from("/root");
    pub fn construct(&self, rt: &mut MockRuntime) {
        rt.expect_validate_caller_addr(vec![*SYSTEM_ACTOR_ADDR]);
        let params = ConstructorParams {
            network_name: nn,
            checkpoint_period: 10, 
        };
        rt.call::<SCAActor>(Method::Constructor as MethodNum, &RawBytes::serialize(params).unwrap()).unwrap();
        rt.verify()
    }

    pub fn construct_and_verify(&self, rt: &mut MockRuntime) {
        self.construct(rt);
        let st: State = rt.get_state().unwrap();
        assert_eq!(st.network_name, self.nn);
        verify_empty_map(rt, st.subnets);
        verify_empty_map(rt, st.subnets);
    }

}

pub fn verify_empty_map(rt: &MockRuntime, key: Cid) {
    let map =
        make_map_with_root_and_bitwidth::<_, BigIntDe>(&key, &rt.store, HAMT_BIT_WIDTH).unwrap();
    map.for_each(|_key, _val| panic!("expected no keys")).unwrap();
}
