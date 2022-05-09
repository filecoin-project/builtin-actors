use fvm_ipld_hamt::Hamt;
use fvm_shared::clock::NO_QUANTIZATION;
use fil_actor_miner::BitFieldQueue;

mod util;
use util::*;
use fil_actors_runtime::{ make_empty_map };

const TEST_AMT_BITWIDTH: u32 = 3;

#[test]
#[ignore = "todo"]
fn adds_values_to_empty_queue() {
    let mut h = ActorHarness::new(0);
    let mut rt = h.new_runtime();
    let store = rt.store;
    let mut array: Hamt<_, usize> = make_empty_map(&store, TEST_AMT_BITWIDTH);
    let cid = array.flush().unwrap();
    let mut queue = BitFieldQueue::new(&store, &cid, NO_QUANTIZATION).unwrap();
    todo!();
}