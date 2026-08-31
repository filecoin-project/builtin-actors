// Cron drops every deal at the visit scheduled for it at publish, settled in the meantime or
// not. Only deals already past that visit, the legacy ones from pre-FIP-0074, fall through
// to be rescheduled.

use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fil_actors_runtime::runtime::{Policy, Runtime};
use fil_actors_runtime::test_utils::MockRuntime;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::deal::DealID;
use regex::Regex;

use fil_actor_market::{DealProposal, State, deal_cid};

mod harness;
use harness::*;

const START_EPOCH: ChainEpoch = 50;
const END_EPOCH: ChainEpoch = START_EPOCH + 200 * EPOCHS_IN_DAY;
const SECTOR_NUMBER: u64 = 7;

fn scheduled_epochs(rt: &MockRuntime, deal_id: DealID) -> Vec<ChainEpoch> {
    let st: State = rt.get_state();
    let ops = st.load_deal_ops(rt.store()).unwrap();
    let mut epochs = Vec::new();
    ops.for_each(|epoch, _| {
        ops.for_each_in(&epoch, |id| {
            if id == deal_id {
                epochs.push(epoch);
            }
            Ok(())
        })
    })
    .unwrap();
    epochs
}

fn pending(rt: &MockRuntime, proposal: &DealProposal) -> bool {
    let st: State = rt.get_state();
    st.has_pending_deal(rt.store(), &deal_cid(rt, proposal).unwrap()).unwrap()
}

fn cron_tick_no_payment(rt: &MockRuntime) {
    let client = get_balance(rt, &CLIENT_ADDR);
    let provider = get_balance(rt, &PROVIDER_ADDR);
    cron_tick(rt);
    assert_eq!(client, get_balance(rt, &CLIENT_ADDR));
    assert_eq!(provider, get_balance(rt, &PROVIDER_ADDR));
}

fn publish_and_activate(rt: &MockRuntime, start_epoch: ChainEpoch) -> (DealID, DealProposal) {
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    publish_and_activate_deal(
        rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        SECTOR_NUMBER,
        start_epoch,
        end_epoch,
        0,
        end_epoch,
    )
}

#[test]
fn unsettled_deal_is_dropped_at_its_first_visit() {
    let rt = setup();
    let (deal_id, proposal) = publish_and_activate(&rt, START_EPOCH);
    let first_visit = process_epoch(START_EPOCH, deal_id);
    assert_eq!(vec![first_visit], scheduled_epochs(&rt, deal_id));

    rt.set_epoch(first_visit);
    cron_tick(&rt);

    assert!(scheduled_epochs(&rt, deal_id).is_empty());
    assert!(!pending(&rt, &proposal));
    assert_eq!(EPOCH_UNDEFINED, get_deal_state(&rt, deal_id).last_updated_epoch);
    check_state(&rt);
}
#[test]
fn missing_pending_at_first_visit_does_not_block_cron() {
    let rt = setup();
    let (deal_id, proposal) = publish_and_activate(&rt, START_EPOCH);
    let first_visit = process_epoch(START_EPOCH, deal_id);
    let mut state: State = rt.get_state();
    state.remove_pending_deal(rt.store(), deal_cid(&rt, &proposal).unwrap()).unwrap();
    rt.replace_state(&state);

    rt.set_epoch(first_visit);
    cron_tick_no_payment(&rt);

    assert!(scheduled_epochs(&rt, deal_id).is_empty());
    assert!(!pending(&rt, &proposal));
    assert_eq!(EPOCH_UNDEFINED, get_deal_state(&rt, deal_id).last_updated_epoch);
    check_state(&rt);
}

#[test]
fn invariant_rejects_a_never_visited_deal_at_the_wrong_epoch() {
    let rt = setup();
    let (deal_id, _) = publish_and_activate(&rt, START_EPOCH);
    let first_visit = process_epoch(START_EPOCH, deal_id);
    let interval = rt.policy.deal_updates_interval;
    let mut state: State = rt.get_state();
    let mut deal_ops = state.load_deal_ops(rt.store()).unwrap();
    // Cron is at epoch 0; misqueue the deal one interval past its required first visit.
    deal_ops.remove(&first_visit, deal_id).unwrap();
    deal_ops.put(&(first_visit + interval), deal_id).unwrap();
    state.deal_ops_by_epoch = deal_ops.flush().unwrap();
    rt.replace_state(&state);

    check_state_with_expected(
        &rt,
        &[Regex::new("never-visited deal .* exactly one deal op at epoch").unwrap()],
    );
}

#[test]
fn deal_settled_before_its_first_visit_is_dropped_at_it() {
    let rt = setup();
    let (deal_id, proposal) = publish_and_activate(&rt, START_EPOCH);
    let first_visit = process_epoch(START_EPOCH, deal_id);
    assert!(first_visit > START_EPOCH + 1);

    // settlement after the start epoch clears the pending proposal itself
    rt.set_epoch(START_EPOCH + 1);
    settle_deal_payments(&rt, PROVIDER_ADDR, &[deal_id], &[], &[]);
    assert!(!pending(&rt, &proposal));
    assert_eq!(START_EPOCH + 1, get_deal_state(&rt, deal_id).last_updated_epoch);
    assert_eq!(vec![first_visit], scheduled_epochs(&rt, deal_id));

    rt.set_epoch(first_visit);
    cron_tick_no_payment(&rt);

    assert!(scheduled_epochs(&rt, deal_id).is_empty());
    assert_eq!(START_EPOCH + 1, get_deal_state(&rt, deal_id).last_updated_epoch);
    check_state(&rt);
}

#[test]
fn deal_settled_at_its_start_epoch_is_dropped_at_its_first_visit() {
    let rt = setup();
    let (deal_id, proposal) = publish_and_activate(&rt, START_EPOCH);
    let first_visit = process_epoch(START_EPOCH, deal_id);

    // settlement at the start epoch leaves the pending proposal for cron
    rt.set_epoch(START_EPOCH);
    settle_deal_payments(&rt, PROVIDER_ADDR, &[deal_id], &[], &[]);
    assert!(pending(&rt, &proposal));
    assert_eq!(START_EPOCH, get_deal_state(&rt, deal_id).last_updated_epoch);

    rt.set_epoch(first_visit);
    cron_tick(&rt);

    assert!(scheduled_epochs(&rt, deal_id).is_empty());
    assert!(!pending(&rt, &proposal));
    check_state(&rt);
}

#[test]
fn deal_whose_first_visit_is_its_start_epoch_is_dropped_there() {
    let rt = setup();
    // deal 0 starting on an interval boundary is scheduled at its own start epoch
    let start_epoch = Policy::default().deal_updates_interval;
    let (deal_id, proposal) = publish_and_activate(&rt, start_epoch);
    assert_eq!(0, deal_id);
    assert_eq!(start_epoch, process_epoch(start_epoch, deal_id));

    rt.set_epoch(start_epoch);
    settle_deal_payments(&rt, PROVIDER_ADDR, &[deal_id], &[], &[]);
    cron_tick(&rt);

    assert!(scheduled_epochs(&rt, deal_id).is_empty());
    assert!(!pending(&rt, &proposal));
    check_state(&rt);
}

#[test]
fn first_visit_is_recognised_when_cron_catches_up_late() {
    let rt = setup();
    let (deal_id, _) = publish_and_activate(&rt, START_EPOCH);
    let first_visit = process_epoch(START_EPOCH, deal_id);

    rt.set_epoch(START_EPOCH + 1);
    settle_deal_payments(&rt, PROVIDER_ADDR, &[deal_id], &[], &[]);

    // cron has not run since before the deal was published, so it walks over the first visit
    // from a later epoch
    rt.set_epoch(first_visit + 3);
    cron_tick_no_payment(&rt);

    assert!(scheduled_epochs(&rt, deal_id).is_empty());
    check_state(&rt);
}

#[test]
fn dropped_deal_is_never_revisited_and_still_settles_manually() {
    let rt = setup();
    let (deal_id, proposal) = publish_and_activate(&rt, START_EPOCH);
    let first_visit = process_epoch(START_EPOCH, deal_id);
    let interval = Policy::default().deal_updates_interval;

    rt.set_epoch(START_EPOCH + 1);
    settle_deal_payments(&rt, PROVIDER_ADDR, &[deal_id], &[], &[]);

    rt.set_epoch(first_visit);
    cron_tick_no_payment(&rt);
    rt.set_epoch(first_visit + interval);
    cron_tick_no_change(&rt, CLIENT_ADDR, PROVIDER_ADDR);
    rt.set_epoch(END_EPOCH + 100);
    cron_tick_no_change(&rt, CLIENT_ADDR, PROVIDER_ADDR);

    expect_emitted(
        &rt,
        "deal-completed",
        deal_id,
        proposal.client.id().unwrap(),
        proposal.provider.id().unwrap(),
    );
    let ret = settle_deal_payments(&rt, PROVIDER_ADDR, &[deal_id], &[], &[]);
    assert!(ret.settlements[0].completed);
    assert_deal_deleted(&rt, deal_id, &proposal, SECTOR_NUMBER, true);
    check_state(&rt);
}

#[test]
fn legacy_deal_is_rescheduled_at_its_visit() {
    let rt = setup();
    let (deal_id, _) = publish_and_activate_deal_legacy(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        SECTOR_NUMBER,
        START_EPOCH,
        END_EPOCH,
        0,
        END_EPOCH,
    );
    let visit = legacy_process_epoch(START_EPOCH, deal_id);
    let interval = Policy::default().deal_updates_interval;
    assert_eq!(vec![visit], scheduled_epochs(&rt, deal_id));

    rt.set_epoch(visit);
    cron_tick(&rt);

    assert_eq!(vec![visit + interval], scheduled_epochs(&rt, deal_id));
    assert_eq!(visit, get_deal_state(&rt, deal_id).last_updated_epoch);
    check_state(&rt);
}

#[test]
fn termination_before_first_visit_clears_pending_kept_by_settlement_at_start() {
    let rt = setup();
    let (deal_id, proposal) = publish_and_activate(&rt, START_EPOCH);

    rt.set_epoch(START_EPOCH);
    settle_deal_payments(&rt, PROVIDER_ADDR, &[deal_id], &[], &[]);
    assert!(pending(&rt, &proposal));

    rt.set_epoch(START_EPOCH + 5);
    terminate_deals(&rt, PROVIDER_ADDR, &[SECTOR_NUMBER], &[deal_id]);

    assert!(!pending(&rt, &proposal));
    assert_deal_deleted(&rt, deal_id, &proposal, SECTOR_NUMBER, true);
    check_state(&rt);
}
