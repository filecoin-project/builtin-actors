use fil_actor_miner::pledge_penalty_for_continued_fault;
use fil_actor_miner::power_for_sectors;
use fil_actors_runtime::test_utils::expect_abort_contains_message;
use fil_actors_runtime::test_utils::MockRuntime;
use fvm_ipld_bitfield::BitField;
use fvm_shared::address::Address;

use fvm_shared::clock::ChainEpoch;
use fvm_shared::consensus::{ConsensusFault, ConsensusFaultType};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;

use num_traits::Zero;

mod util;
use crate::util::*;

const PERIOD_OFFSET: ChainEpoch = 100;

#[test]
fn recovery_happy_path() {
    let (mut h, mut rt) = setup();
    let one_sector =
        h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);

    // advance to first proving period and submit so we'll have time to declare the fault next cycle
    h.advance_and_submit_posts(&mut rt, &one_sector);

    // Declare the sector as faulted
    h.declare_faults(&mut rt, &one_sector);

    // Declare recoveries updates state
    let st = h.get_state(&rt);
    let (dl_idx, p_idx) =
        st.find_sector(&rt.policy, &rt.store, one_sector[0].sector_number).unwrap();
    h.declare_recoveries(
        &mut rt,
        dl_idx,
        p_idx,
        BitField::try_from_bits([one_sector[0].sector_number]).unwrap(),
        TokenAmount::zero(),
    )
    .unwrap();

    let dl = h.get_deadline(&rt, dl_idx);
    let p = dl.load_partition(&rt.store, p_idx).unwrap();
    assert_eq!(p.faults, p.recoveries);
    h.check_state(&rt);
}

#[test]
fn recovery_must_pay_back_fee_debt() {
    let (mut h, mut rt) = setup();
    let one_sector =
        h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);
    // advance to first proving period and submit so we'll have time to declare the fault next cycle
    h.advance_and_submit_posts(&mut rt, &one_sector);

    // Fault will take miner into fee debt
    let mut st = h.get_state(&rt);
    rt.set_balance(&st.pre_commit_deposits + &st.initial_pledge + &st.locked_funds);

    h.declare_faults(&mut rt, &one_sector);

    st = h.get_state(&rt);
    let (dl_idx, p_idx) =
        st.find_sector(&rt.policy, &rt.store, one_sector[0].sector_number).unwrap();

    // Skip to end of proving period
    h.advance_to_deadline(&mut rt, dl_idx);

    // Can't pay during this deadline so miner goes into fee debt
    let ongoing_pwr = power_for_sectors(h.sector_size, &one_sector);
    let ff = pledge_penalty_for_continued_fault(
        &h.epoch_reward_smooth,
        &h.epoch_qa_power_smooth,
        &ongoing_pwr.qa,
    );
    h.advance_deadline(
        &mut rt,
        CronConfig {
            continued_faults_penalty: TokenAmount::zero(), // fee is instead added to debt
            ..Default::default()
        },
    );

    st = h.get_state(&rt);
    assert_eq!(ff, st.fee_debt);

    // Recovery fails when it can't pay back fee debt
    expect_abort_contains_message(
        ExitCode::USR_INSUFFICIENT_FUNDS,
        "unlocked balance can not repay fee debt",
        h.declare_recoveries(
            &mut rt,
            dl_idx,
            p_idx,
            BitField::try_from_bits([one_sector[0].sector_number]).unwrap(),
            TokenAmount::zero(),
        ),
    );

    // Recovery pays back fee debt and succeeds
    let funds = &ff + st.initial_pledge + st.locked_funds + st.pre_commit_deposits;
    rt.set_balance(funds);
    h.declare_recoveries(
        &mut rt,
        dl_idx,
        p_idx,
        BitField::try_from_bits([one_sector[0].sector_number]).unwrap(),
        ff,
    )
    .unwrap();

    let dl = h.get_deadline(&rt, dl_idx);
    let p = dl.load_partition(&rt.store, p_idx).unwrap();
    assert_eq!(p.faults, p.recoveries);
    st = h.get_state(&rt);
    assert!(st.fee_debt.is_zero());
    h.check_state(&rt);
}

#[test]
fn recovery_fails_during_active_consensus_fault() {
    let (mut h, mut rt) = setup();
    let one_sector =
        h.commit_and_prove_sectors(&mut rt, 1, DEFAULT_SECTOR_EXPIRATION as u64, vec![], true);

    // consensus fault
    let test_addr = Address::new_actor("satoshi".as_bytes());
    let epoch = rt.epoch;
    h.report_consensus_fault(
        &mut rt,
        test_addr,
        Some(ConsensusFault {
            target: h.receiver,
            epoch: epoch - 1,
            fault_type: ConsensusFaultType::DoubleForkMining,
        }),
    )
    .unwrap();

    // advance to first proving period and submit so we'll have time to declare the fault next cycle
    h.advance_and_submit_posts(&mut rt, &one_sector);

    // Declare the sector as faulted
    h.declare_faults(&mut rt, &one_sector);
    let st = h.get_state(&rt);
    let (dl_idx, p_idx) =
        st.find_sector(&rt.policy, &rt.store, one_sector[0].sector_number).unwrap();
    expect_abort_contains_message(
        ExitCode::USR_FORBIDDEN,
        "recovery not allowed during active consensus fault",
        h.declare_recoveries(
            &mut rt,
            dl_idx,
            p_idx,
            BitField::try_from_bits([one_sector[0].sector_number]).unwrap(),
            TokenAmount::zero(),
        ),
    );
    h.check_state(&rt);
}

fn setup() -> (ActorHarness, MockRuntime) {
    let h = ActorHarness::new(PERIOD_OFFSET);
    let mut rt = h.new_runtime();
    h.construct_and_verify(&mut rt);
    rt.set_balance(BIG_BALANCE.clone());

    (h, rt)
}
