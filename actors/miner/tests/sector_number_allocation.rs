// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::test_utils::*;
use fvm_shared::clock::ChainEpoch;

use fvm_shared::error::ExitCode;
use fvm_shared::sector::MAX_SECTOR_NUMBER;

mod util;
use util::*;

mod state_harness;
use state_harness::*;

const PERIOD_OFFSET: ChainEpoch = 0;

mod sector_number_allocation {
    use super::*;

    #[test]
    fn batch_allocation() {
        let policy = Policy::default();
        let mut h = StateHarness::new_with_policy(&policy, PERIOD_OFFSET);
        h.allocate(&[1, 2, 3]).unwrap();
        h.allocate(&[4, 5, 6]).unwrap();
        h.expect(&bitfield_from_slice(&[1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn repeat_allocation_rejected() {
        let policy = Policy::default();
        let mut h = StateHarness::new_with_policy(&policy, PERIOD_OFFSET);
        h.allocate(&[1]).unwrap();
        assert!(h.allocate(&[1]).is_err());
        h.expect(&bitfield_from_slice(&[1]));
    }

    #[test]
    fn overlapping_batch_rejected() {
        let policy = Policy::default();
        let mut h = StateHarness::new_with_policy(&policy, PERIOD_OFFSET);
        h.allocate(&[1, 2, 3]).unwrap();
        assert!(h.allocate(&[3, 4, 5]).is_err());
        h.expect(&bitfield_from_slice(&[1, 2, 3]));
    }

    #[test]
    fn batch_masking() {
        let policy = Policy::default();
        let mut h = StateHarness::new_with_policy(&policy, PERIOD_OFFSET);
        h.allocate(&[1]).unwrap();

        h.mask(&bitfield_from_slice(&[0, 1, 2, 3])).unwrap();
        h.expect(&bitfield_from_slice(&[0, 1, 2, 3]));

        assert!(h.allocate(&[0]).is_err());
        assert!(h.allocate(&[3]).is_err());
        h.allocate(&[4]).unwrap();
        h.expect(&bitfield_from_slice(&[0, 1, 2, 3, 4]));
    }

    #[test]
    fn range_limits() {
        let policy = Policy::default();
        let mut h = StateHarness::new_with_policy(&policy, PERIOD_OFFSET);

        h.allocate(&[0]).unwrap();
        h.allocate(&[MAX_SECTOR_NUMBER]).unwrap();
        h.expect(&bitfield_from_slice(&[0, MAX_SECTOR_NUMBER]));
    }

    #[test]
    fn mask_range_limits() {
        let policy = Policy::default();
        let mut h = StateHarness::new_with_policy(&policy, PERIOD_OFFSET);

        h.mask(&bitfield_from_slice(&[0])).unwrap();
        h.mask(&bitfield_from_slice(&[MAX_SECTOR_NUMBER])).unwrap();
        h.expect(&bitfield_from_slice(&[0, MAX_SECTOR_NUMBER]));
    }

    #[test]
    fn compaction_with_mask() {
        let policy = Policy::default();
        let mut h = StateHarness::new_with_policy(&policy, PERIOD_OFFSET);

        // Allocate widely-spaced numbers to consume the run-length encoded bytes quickly,
        // until the limit is reached.
        let mut limit_reached = false;
        for i in 0..std::u64::MAX {
            let (number, _) = (i + 1).overflowing_shl(50);
            let res = h.allocate(&[number]);
            if res.is_err() {
                // We failed, yay!
                limit_reached = true;
                expect_abort(ExitCode::USR_SERIALIZATION, res);

                // mask half the sector ranges.
                let to_mask = seq(0, number / 2);
                h.mask(&to_mask).unwrap();

                // try again
                h.allocate(&[number]).unwrap();
                break;
            }
        }
        assert!(limit_reached);
    }
}
