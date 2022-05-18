// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actor_miner::BitFieldQueue;
use fil_actors_runtime::runtime::Policy;
use fvm_ipld_blockstore::Blockstore;
use fvm_shared::clock::ChainEpoch;

use std::collections::BTreeMap;

mod util;
use util::*;

mod state_harness;
use state_harness::*;

const PERIOD_OFFSET: ChainEpoch = 0;

#[derive(Default, Clone)]
pub struct BitfieldQueueExpectation {
    pub expected: BTreeMap<ChainEpoch, Vec<u64>>,
}

impl BitfieldQueueExpectation {
    pub fn add(&self, epoch: ChainEpoch, values: &[u64]) -> Self {
        let mut expected = self.expected.clone();
        let _ = expected.insert(epoch, values.to_vec());
        BitfieldQueueExpectation { expected }
    }

    pub fn equals<BS: Blockstore>(&self, queue: BitFieldQueue<BS>) {
        // ensure cached changes are ready to be iterated

        let length = queue.amt.count();
        assert_eq!(self.expected.len(), length as usize);

        queue
            .amt
            .for_each(|epoch, bf| {
                let values = self
                    .expected
                    .get(&(epoch as i64))
                    .unwrap_or_else(|| panic!("expected entry at epoch {}", epoch));

                assert_bitfield_equals(bf, values);
                Ok(())
            })
            .unwrap();
    }
}

mod add_precommit_expiry {
    use super::*;

    #[test]
    fn simple_pre_commit_expiry_and_cleanup() {
        let policy = Policy::default();
        let mut h = StateHarness::new_with_policy(&policy, PERIOD_OFFSET);

        h.add_pre_commit_clean_ups(&policy, Vec::from([(100, 1)])).unwrap();

        let quant = h.quant_spec_every_deadline(&policy);
        BitfieldQueueExpectation::default()
            .add(quant.quantize_up(100), &[1])
            .equals(h.load_pre_commit_clean_ups(&policy));

        h.add_pre_commit_clean_ups(&policy, Vec::from([(100, 2)])).unwrap();
        BitfieldQueueExpectation::default()
            .add(quant.quantize_up(100), &[1, 2])
            .equals(h.load_pre_commit_clean_ups(&policy));

        h.add_pre_commit_clean_ups(&policy, Vec::from([(200, 3)])).unwrap();
        BitfieldQueueExpectation::default()
            .add(quant.quantize_up(100), &[1, 2])
            .add(quant.quantize_up(200), &[3])
            .equals(h.load_pre_commit_clean_ups(&policy));
    }

    #[test]
    fn batch_pre_commit_expiry() {
        let policy = Policy::default();
        let mut h = StateHarness::new_with_policy(&policy, PERIOD_OFFSET);

        h.add_pre_commit_clean_ups(&policy, Vec::from([(100, 1), (200, 2), (200, 3)])).unwrap();

        let quant = h.quant_spec_every_deadline(&policy);
        BitfieldQueueExpectation::default()
            .add(quant.quantize_up(100), &[1])
            .add(quant.quantize_up(200), &[2, 3])
            .equals(h.load_pre_commit_clean_ups(&policy));

        h.add_pre_commit_clean_ups(
            &policy,
            Vec::from([
                (100, 1), // Redundant
                (200, 4),
                (300, 5),
                (300, 6),
            ]),
        )
        .unwrap();
        BitfieldQueueExpectation::default()
            .add(quant.quantize_up(100), &[1])
            .add(quant.quantize_up(200), &[2, 3, 4])
            .add(quant.quantize_up(300), &[5, 6])
            .equals(h.load_pre_commit_clean_ups(&policy));
    }
}
