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

#[derive(Default)]
pub struct BQExpectation {
    expected: HashMap<ChainEpoch, Vec<u64>>,
}

impl BQExpectation {
    fn add(&mut self, epoch: ChainEpoch, values: Vec<u64>) -> &mut BQExpectation {
        self.expected.entry(epoch).or_insert(values);
        self
    }

    fn equals(&mut self, q: &mut BitFieldQueue<MemoryBlockstore>) {
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

    let mut bq_expectation = BQExpectation::default();
    bq_expectation.add(epoch, values.to_vec()).equals(&mut queue);
}

#[test]
fn adds_bitfield_to_empty_queue() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let mut queue = empty_bitfield_queue(&rt, TEST_AMT_BITWIDTH);

    let values = BitField::from_ranges(Ranges::new([1..5]));
    let epoch = ChainEpoch::from(42);

    queue.add_to_queue(epoch, &values).unwrap();

    let mut bq_expectation = BQExpectation::default();
    bq_expectation.add(epoch, values.iter().collect()).equals(&mut queue);
}

#[test]
fn quantizes_added_epochs_according_to_quantization_spec() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let mut queue = empty_bitfield_queue_with_quantizing(
        &rt,
        QuantSpec { unit: 5, offset: 3 },
        TEST_AMT_BITWIDTH,
    );

    let range: Vec<u64> = vec![0, 2, 3, 4, 7, 8, 9];
    for val in range {
        queue.add_to_queue_values(val as i64, [val]).unwrap();
    }

    let mut bq_expectation = BQExpectation::default();
    // expect values to only be set on quantization boundaries
    bq_expectation
        .add(ChainEpoch::from(3), [0, 2, 3].to_vec())
        .add(ChainEpoch::from(8), [4, 7, 8].to_vec())
        .add(ChainEpoch::from(13), [9].to_vec())
        .equals(&mut queue);
}

#[test]
fn merges_values_within_same_epoch() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let mut queue = empty_bitfield_queue(&rt, TEST_AMT_BITWIDTH);

    let epoch = ChainEpoch::from(42);

    queue.add_to_queue_values(epoch, [1, 3].to_vec()).unwrap();
    queue.add_to_queue_values(epoch, [2, 4].to_vec()).unwrap();

    let mut bq_expectation = BQExpectation::default();
    bq_expectation.add(epoch, [1, 2, 3, 4].to_vec()).equals(&mut queue);
}

#[test]
fn adds_values_to_different_epochs() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let mut queue = empty_bitfield_queue(&rt, TEST_AMT_BITWIDTH);

    let epoch1 = ChainEpoch::from(42);
    let epoch2 = ChainEpoch::from(93);

    queue.add_to_queue_values(epoch1, [1, 3].to_vec()).unwrap();
    queue.add_to_queue_values(epoch2, [2, 4].to_vec()).unwrap();

    let mut bq_expectation = BQExpectation::default();
    bq_expectation.add(epoch1, [1, 3].to_vec()).add(epoch2, [2, 4].to_vec()).equals(&mut queue);
}

#[test]
fn pop_until_from_empty_queue_returns_empty_bitfield() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let mut queue = empty_bitfield_queue(&rt, TEST_AMT_BITWIDTH);

    let (next, modified) = queue.pop_until(42).unwrap();

    //no values are returned
    let count = next.len();
    assert_eq!(0, count);
    // modified is false
    assert!(!modified);
}

#[test]
fn pop_until_does_nothing_if_until_parameter_before_first_value() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let mut queue = empty_bitfield_queue(&rt, TEST_AMT_BITWIDTH);

    let epoch1 = ChainEpoch::from(42);
    let epoch2 = ChainEpoch::from(93);

    queue.add_to_queue_values(epoch1, [1, 3].to_vec()).unwrap();
    queue.add_to_queue_values(epoch2, [2, 4].to_vec()).unwrap();

    let (next, modified) = queue.pop_until(epoch1 - 1).unwrap();

    //no values are returned
    let count = next.len();
    assert_eq!(0, count);
    // modified is false
    assert!(!modified);

    let mut bq_expectation = BQExpectation::default();
    // queue remains the same
    bq_expectation.add(epoch1, [1, 3].to_vec()).add(epoch2, [2, 4].to_vec()).equals(&mut queue);
}

#[test]
fn pop_until_removes_and_returns_entries_before_and_including_target_epoch() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let mut queue = empty_bitfield_queue(&rt, TEST_AMT_BITWIDTH);

    let epoch1 = ChainEpoch::from(42);
    let epoch2 = ChainEpoch::from(93);
    let epoch3 = ChainEpoch::from(94);
    let epoch4 = ChainEpoch::from(204);

    queue.add_to_queue_values(epoch1, [1, 3].to_vec()).unwrap();
    queue.add_to_queue_values(epoch2, [5].to_vec()).unwrap();
    queue.add_to_queue_values(epoch3, [6, 7, 8].to_vec()).unwrap();
    queue.add_to_queue_values(epoch4, [2, 4].to_vec()).unwrap();

    let (next, modified) = queue.pop_until(epoch2).unwrap();
    // modified should be true to indicate queue has changed
    assert!(modified);

    // values from first two epochs are returned
    assert_bitfield_equals(&next, &[1, 3, 5]);

    let mut bq_expectation = BQExpectation::default();
    // queue only contains remaining values
    bq_expectation.add(epoch3, [6, 7, 8].to_vec()).add(epoch4, [2, 4].to_vec()).equals(&mut queue);

    // subsequent call to epoch less than next does nothing
    let (next, modified) = queue.pop_until(epoch3 - 1).unwrap();
    assert!(!modified);

    // no values are returned
    assert_bitfield_equals(&next, &[]);

    let mut bq_expectation = BQExpectation::default();
    // queue only contains remaining values
    bq_expectation.add(epoch3, [6, 7, 8].to_vec()).add(epoch4, [2, 4].to_vec()).equals(&mut queue);

    // popping the rest of the queue gets the rest of the values
    let (next, modified) = queue.pop_until(epoch4).unwrap();
    assert!(modified);

    // rest of values are returned
    assert_bitfield_equals(&next, &[2, 4, 6, 7, 8]);

    let mut bq_expectation = BQExpectation::default();
    // queue is now empty
    bq_expectation.equals(&mut queue);
}

#[test]
fn cuts_elements() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let mut queue = empty_bitfield_queue(&rt, TEST_AMT_BITWIDTH);

    let epoch1 = ChainEpoch::from(42);
    let epoch2 = ChainEpoch::from(93);

    queue.add_to_queue_values(epoch1, [1, 2, 3, 4, 99].to_vec()).unwrap();
    queue.add_to_queue_values(epoch2, [5, 6].to_vec()).unwrap();

    let ranges: Vec<Range<u64>> = vec![Range { start: 2, end: 3 }, Range { start: 4, end: 7 }];
    let to_cut = BitField::from_ranges(Ranges::new(ranges.iter().cloned()));
    queue.cut(&to_cut).unwrap();

    let mut bq_expectation = BQExpectation::default();
    // 3 shifts down to 2, 99 down to 95
    bq_expectation.add(epoch1, [1, 2, 95].to_vec()).equals(&mut queue);
}

#[test]
fn adds_empty_bitfield_to_queue() {
    let h = ActorHarness::new(0);
    let rt = h.new_runtime();
    let mut queue = empty_bitfield_queue(&rt, TEST_AMT_BITWIDTH);

    let epoch = ChainEpoch::from(42);

    queue.add_to_queue(epoch, &BitField::new()).unwrap();

    let mut bq_expectation = BQExpectation::default();
    // ensures we don't add an empty entry
    bq_expectation.equals(&mut queue);
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
