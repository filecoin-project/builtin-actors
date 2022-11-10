use fil_actors_runtime_common::test_utils::{expect_abort, MockRuntime};
use fvm_shared::address::Address;
use fvm_shared::sector::MAX_SECTOR_NUMBER;
use fvm_shared::{clock::ChainEpoch, error::ExitCode};

mod util;
use util::*;

const PERIOD_OFFSET: ChainEpoch = 100;

fn setup() -> (ActorHarness, MockRuntime) {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);
    rt.balance.replace(BIG_BALANCE.clone());

    (h, rt)
}

mod compact_sector_numbers_test {
    use super::*;

    #[test]
    fn compact_sector_numbers_then_pre_commit() {
        // Create a sector.
        let (mut h, mut rt) = setup();
        let all_sectors =
            h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![], true);

        let target_sector_num = all_sectors[0].sector_number;
        h.compact_sector_numbers(
            &mut rt,
            h.worker,
            bitfield_from_slice(&[target_sector_num, target_sector_num + 1]),
        );

        let precommit_epoch = rt.epoch;
        let deadline = h.deadline(&rt);
        let expiration = deadline.period_end()
            + DEFAULT_SECTOR_EXPIRATION as i64 * rt.policy.wpost_proving_period;

        // Allocating masked sector number should fail.
        {
            let precommit = h.make_pre_commit_params(
                target_sector_num + 1,
                precommit_epoch - 1,
                expiration,
                vec![],
            );
            expect_abort(
                ExitCode::USR_ILLEGAL_ARGUMENT,
                h.pre_commit_sector(&mut rt, precommit, util::PreCommitConfig::default(), false),
            );
        }

        {
            let precommit = h.make_pre_commit_params(
                target_sector_num + 2,
                precommit_epoch - 1,
                expiration,
                vec![],
            );
            h.pre_commit_sector(&mut rt, precommit, util::PreCommitConfig::default(), false)
                .unwrap();
        }
        check_state_invariants_from_mock_runtime(&rt);
    }

    #[test]
    fn owner_can_also_compact_sectors() {
        // Create a sector.
        let (mut h, mut rt) = setup();
        let all_sectors =
            h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![], true);

        let target_sector_num = all_sectors[0].sector_number;
        h.compact_sector_numbers(
            &mut rt,
            h.owner,
            bitfield_from_slice(&[target_sector_num, target_sector_num + 1]),
        );
        check_state_invariants_from_mock_runtime(&rt);
    }

    #[test]
    fn one_of_the_control_addresses_can_also_compact_sectors() {
        // Create a sector.
        let (mut h, mut rt) = setup();
        let all_sectors =
            h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![], true);

        let target_sector_num = all_sectors[0].sector_number;
        h.compact_sector_numbers(
            &mut rt,
            h.caller_addrs()[0],
            bitfield_from_slice(&[target_sector_num, target_sector_num + 1]),
        );
        check_state_invariants_from_mock_runtime(&rt);
    }

    #[test]
    fn fail_if_caller_is_not_among_caller_worker_or_control_addresses() {
        // Create a sector.
        let (mut h, mut rt) = setup();
        let all_sectors =
            h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION, vec![], true);

        let target_sector_num = all_sectors[0].sector_number;
        let r_addr = Address::new_id(1005);

        expect_abort(
            ExitCode::USR_FORBIDDEN,
            h.compact_sector_numbers_raw(
                &mut rt,
                r_addr,
                bitfield_from_slice(&[target_sector_num, target_sector_num + 1]),
            ),
        );

        check_state_invariants_from_mock_runtime(&rt);
    }

    #[test]
    fn sector_number_range_limits() {
        let (h, mut rt) = setup();
        // Limits ok
        h.compact_sector_numbers(&mut rt, h.worker, bitfield_from_slice(&[0, MAX_SECTOR_NUMBER]));

        // Out of range fails
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            h.compact_sector_numbers_raw(
                &mut rt,
                h.worker,
                bitfield_from_slice(&[MAX_SECTOR_NUMBER + 1]),
            ),
        );
        rt.reset();
        check_state_invariants_from_mock_runtime(&rt);
    }

    #[test]
    fn compacting_no_sector_numbers_aborts() {
        let (h, mut rt) = setup();
        expect_abort(
            ExitCode::USR_ILLEGAL_ARGUMENT,
            // compact nothing
            h.compact_sector_numbers_raw(&mut rt, h.worker, bitfield_from_slice(&[])),
        );
        rt.reset();
        check_state_invariants_from_mock_runtime(&rt);
    }
}
