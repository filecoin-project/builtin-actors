// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::runtime::Policy;
use fvm_shared::clock::ChainEpoch;

mod util;
use util::*;

mod state_harness;
use state_harness::*;

const PERIOD_OFFSET: ChainEpoch = 0;

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
            .equals(&h.load_pre_commit_clean_ups(&policy));

        h.add_pre_commit_clean_ups(&policy, Vec::from([(100, 2)])).unwrap();
        BitfieldQueueExpectation::default()
            .add(quant.quantize_up(100), &[1, 2])
            .equals(&h.load_pre_commit_clean_ups(&policy));

        h.add_pre_commit_clean_ups(&policy, Vec::from([(200, 3)])).unwrap();
        BitfieldQueueExpectation::default()
            .add(quant.quantize_up(100), &[1, 2])
            .add(quant.quantize_up(200), &[3])
            .equals(&h.load_pre_commit_clean_ups(&policy));
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
            .equals(&h.load_pre_commit_clean_ups(&policy));

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
            .equals(&h.load_pre_commit_clean_ups(&policy));
    }
}
