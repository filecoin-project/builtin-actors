use fil_actor_miner::BitFieldQueue;
//use fil_actor_power::epoch_key;
use fvm_ipld_amt::Amt;
use fvm_ipld_bitfield::iter::Ranges;
use fvm_ipld_bitfield::BitField;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::clock::NO_QUANTIZATION;
use std::ops::Range;
//use fvm_ipld_encoding::RawBytes;
use fvm_ipld_hamt::Hamt;
use std::convert::TryInto;

mod util;
//use fil_actors_runtime::make_empty_map;
use util::*;

const TEST_AMT_BITWIDTH: u32 = 3;

#[test]
#[ignore = "todo"]
fn adds_values_to_empty_queue() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let store = rt.store;
    let cid = Amt::<(), _>::new_with_bit_width(&store, TEST_AMT_BITWIDTH).flush().unwrap();
    let mut queue = BitFieldQueue::new(&store, &cid, NO_QUANTIZATION).unwrap();

    let values = [1, 2, 3, 4];
    let epoch = ChainEpoch::from(42);

    queue.add_to_queue_values(epoch, values).unwrap();

    let mut bq_expectation: Hamt<_, _, usize> = Hamt::new(&store);
    //println!("{:?}", bq_expectation);
    bq_expectation.set_if_absent(epoch.try_into().unwrap(), values).unwrap();
    //println!("{:?}", bq_expectation.get(&epoch.try_into().unwrap()).unwrap().unwrap());
    let bq_length = bq_expectation.get(&epoch.try_into().unwrap()).unwrap().unwrap().len();
    //println!("{:?}", bq_length);
    //let popped = queue.pop_until(epoch);
    //println!("{:?}", popped.unwrap().0);
    todo!();
}

#[test]
#[ignore = "todo"]
fn adds_bitfield_to_empty_queue() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let store = rt.store;
    let cid = Amt::<(), _>::new_with_bit_width(&store, TEST_AMT_BITWIDTH).flush().unwrap();
    let mut queue = BitFieldQueue::new(&store, &cid, NO_QUANTIZATION).unwrap();

    let mut ranges: Vec<Range<u64>> = Vec::new();
    ranges.push(Range { start: 1, end: 4 });
    let values = BitField::from_ranges(Ranges::new(ranges.iter().cloned()));
    let epoch = ChainEpoch::from(42);

    queue.add_to_queue(epoch, &values).unwrap();
    todo!();
}
