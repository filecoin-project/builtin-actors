use fil_actor_miner::testing::check_state_invariants;
use fil_actors_runtime::runtime::{Runtime, RuntimePolicy};
use fil_actors_runtime::test_utils::{expect_abort, expect_abort_contains_message, MockRuntime};
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::consensus::{ConsensusFault, ConsensusFaultType};

use fvm_shared::error::ExitCode;

mod util;

use util::*;

const PERIOD_OFFSET: ChainEpoch = 100;

fn setup() -> (ActorHarness, MockRuntime) {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    rt.set_balance(BIG_BALANCE.clone());
    h.construct_and_verify(&mut rt);
    (h, rt)
}

#[test]
fn invalid_report_rejected() {
    let (h, mut rt) = setup();
    rt.set_epoch(1);

    let test_addr = Address::new_actor("satoshi".as_bytes());
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.report_consensus_fault(&mut rt, test_addr, None),
    );
    check_state_invariants(rt.policy(), &h.get_state(&rt), rt.store(), &rt.get_balance());
}

#[test]
fn mistargeted_report_rejected() {
    let (h, mut rt) = setup();
    rt.set_epoch(1);

    let test_addr = Address::new_actor("satoshi".as_bytes());
    let epoch = rt.epoch;
    expect_abort(
        ExitCode::USR_ILLEGAL_ARGUMENT,
        h.report_consensus_fault(
            &mut rt,
            test_addr,
            Some(ConsensusFault {
                target: Address::new_id(1234), // Not receiver
                epoch: epoch - 1,
                fault_type: ConsensusFaultType::DoubleForkMining,
            }),
        ),
    );
    check_state_invariants(rt.policy(), &h.get_state(&rt), rt.store(), &rt.get_balance());
}

#[test]
fn report_consensus_fault_pays_reward_and_charges_fee() {
    let (h, mut rt) = setup();
    rt.set_epoch(1);

    let test_addr = Address::new_actor("satoshi".as_bytes());
    let epoch = rt.epoch;
    let receiver = rt.receiver;
    h.report_consensus_fault(
        &mut rt,
        test_addr,
        Some(ConsensusFault {
            target: receiver,
            epoch: epoch - 1,
            fault_type: ConsensusFaultType::DoubleForkMining,
        }),
    )
    .unwrap();
    check_state_invariants(rt.policy(), &h.get_state(&rt), rt.store(), &rt.get_balance());
}

#[test]
fn report_consensus_fault_updates_consensus_fault_reported_field() {
    let (h, mut rt) = setup();
    rt.set_epoch(1);

    let test_addr = Address::new_actor("satoshi".as_bytes());
    let receiver = rt.receiver;

    let start_info = h.get_info(&rt);
    assert_eq!(-1, start_info.consensus_fault_elapsed);

    let report_epoch = 333;
    rt.set_epoch(report_epoch);

    h.report_consensus_fault(
        &mut rt,
        test_addr,
        Some(ConsensusFault {
            target: receiver,
            epoch: report_epoch - 1,
            fault_type: ConsensusFaultType::DoubleForkMining,
        }),
    )
    .unwrap();
    let end_info = h.get_info(&rt);
    assert_eq!(
        report_epoch + rt.policy.consensus_fault_ineligibility_duration,
        end_info.consensus_fault_elapsed
    );
    check_state_invariants(rt.policy(), &h.get_state(&rt), rt.store(), &rt.get_balance());
}

#[test]
fn double_report_of_consensus_fault_fails() {
    let (h, mut rt) = setup();
    rt.set_epoch(1);

    let test_addr = Address::new_actor("satoshi".as_bytes());
    let receiver = rt.receiver;

    let start_info = h.get_info(&rt);
    assert_eq!(-1, start_info.consensus_fault_elapsed);

    let report_epoch = 333;
    rt.set_epoch(report_epoch);

    let fault1 = rt.epoch - 1;
    h.report_consensus_fault(
        &mut rt,
        test_addr,
        Some(ConsensusFault {
            target: receiver,
            epoch: fault1,
            fault_type: ConsensusFaultType::DoubleForkMining,
        }),
    )
    .unwrap();
    let end_info = h.get_info(&rt);
    assert_eq!(
        report_epoch + rt.policy.consensus_fault_ineligibility_duration,
        end_info.consensus_fault_elapsed
    );

    // same fault can't be reported twice
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "too old",
        h.report_consensus_fault(
            &mut rt,
            test_addr,
            Some(ConsensusFault {
                target: receiver,
                epoch: fault1,
                fault_type: ConsensusFaultType::DoubleForkMining,
            }),
        ),
    );
    rt.reset();

    // new consensus faults are forbidden until original has elapsed
    rt.set_epoch(end_info.consensus_fault_elapsed);
    let fault2 = end_info.consensus_fault_elapsed - 1;
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "too old",
        h.report_consensus_fault(
            &mut rt,
            test_addr,
            Some(ConsensusFault {
                target: receiver,
                epoch: fault2,
                fault_type: ConsensusFaultType::DoubleForkMining,
            }),
        ),
    );
    rt.reset();

    // a new consensus fault can be reported for blocks once original has expired
    rt.set_epoch(end_info.consensus_fault_elapsed + 1);
    let fault3 = end_info.consensus_fault_elapsed;
    h.report_consensus_fault(
        &mut rt,
        test_addr,
        Some(ConsensusFault {
            target: receiver,
            epoch: fault3,
            fault_type: ConsensusFaultType::DoubleForkMining,
        }),
    )
    .unwrap();
    let end_info = h.get_info(&rt);
    assert_eq!(
        rt.epoch + rt.policy.consensus_fault_ineligibility_duration,
        end_info.consensus_fault_elapsed
    );

    // old fault still cannot be reported after fault interval has elapsed
    let fault4 = fault1 + 1;
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "too old",
        h.report_consensus_fault(
            &mut rt,
            test_addr,
            Some(ConsensusFault {
                target: receiver,
                epoch: fault4,
                fault_type: ConsensusFaultType::DoubleForkMining,
            }),
        ),
    );
    check_state_invariants(rt.policy(), &h.get_state(&rt), rt.store(), &rt.get_balance());
}
