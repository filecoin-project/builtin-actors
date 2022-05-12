use fil_actor_miner::BitFieldQueue;
use fil_actors_runtime::test_utils::MockRuntime;
use fvm_ipld_amt::Amt;
use fvm_ipld_bitfield::iter::Ranges;
use fvm_ipld_bitfield::BitField;
use fvm_ipld_blockstore::MemoryBlockstore;
use fvm_shared::clock::{ChainEpoch, QuantSpec, NO_QUANTIZATION};
use std::collections::HashMap;
use std::ops::Range;

mod util;
use util::*;

const TEST_AMT_BITWIDTH: u32 = 3;

pub struct BQExpectation {
    expected: HashMap<ChainEpoch, Vec<u64>>,
}

impl BQExpectation {
    fn add(&mut self, epoch: ChainEpoch, values: Vec<u64>) -> &mut BQExpectation {
        self.expected.entry(epoch).or_insert(values);
        self
    }

    fn equals(&mut self, mut q: BitFieldQueue<MemoryBlockstore>) {
        assert_eq!(self.expected.len() as u64, q.amt.count());

        q.amt
            .for_each_mut(|epoch, bitfield| {
                let values = &self.expected[&(epoch as i64)];
                assert_bitfield_equals(bitfield, values);
                Ok(())
            })
            .unwrap();
    }
}

#[test]
fn adds_values_to_empty_queue() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let mut queue = empty_bitfield_queue(&rt, TEST_AMT_BITWIDTH);

    let values = [1, 2, 3, 4];
    let epoch = ChainEpoch::from(42);

    queue.add_to_queue_values(epoch, values).unwrap();

    let expected: HashMap<_, Vec<u64>> = HashMap::new();
    let mut bq_expectation = BQExpectation { expected };
    bq_expectation.add(epoch, values.to_vec()).equals(queue);
}

#[test]
fn adds_bitfield_to_empty_queue() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let mut queue = empty_bitfield_queue(&rt, TEST_AMT_BITWIDTH);

    let ranges: Vec<Range<u64>> = vec![Range { start: 1, end: 5 }];
    let values = BitField::from_ranges(Ranges::new(ranges.iter().cloned()));
    let epoch = ChainEpoch::from(42);

    queue.add_to_queue(epoch, &values).unwrap();

    let expected: HashMap<_, Vec<u64>> = HashMap::new();
    let mut bq_expectation = BQExpectation { expected };
    bq_expectation.add(epoch, ranges[0].clone().map(u64::from).collect::<Vec<u64>>()).equals(queue);
}

fn empty_bitfield_queue_with_quantizing(
    rt: &MockRuntime,
    quant: QuantSpec,
    bitwidth: u32,
) -> BitFieldQueue<MemoryBlockstore> {
    let cid = Amt::<(), _>::new_with_bit_width(&rt.store, bitwidth).flush().unwrap();

    BitFieldQueue::new(&rt.store, &cid, quant).unwrap()
}

fn empty_bitfield_queue(rt: &MockRuntime, bitwidth: u32) -> BitFieldQueue<MemoryBlockstore> {
    empty_bitfield_queue_with_quantizing(rt, NO_QUANTIZATION, bitwidth)
}
