use fvm_shared::econ::TokenAmount;
use hierarchical_sca::subnet;
use hierarchical_sca::{Actor as SCAActor, State};

use crate::harness::*;

mod harness;

#[test]
fn construct() {
    let mut rt = new_runtime();
    let h = new_harness();
    h.construct_and_verify(&mut rt);
    h.check_state();
}

#[test]
fn register_subnet() {
    let (h, mut rt) = setup();
    h.register(&mut rt, &OWNER, &TokenAmount::from(10_u64.pow(18))).unwrap();
    let st: State = rt.get_state().unwrap();
    assert_eq!(st.total_subnets, 1);
    // Verify the miner's claim.
    let shid = subnet::new_id(&h.net_name, *OWNER);
    let subnet = h.get_subnet(&rt, &shid).unwrap();
    assert_eq!(subnet.id, shid);
    h.check_state();
}
