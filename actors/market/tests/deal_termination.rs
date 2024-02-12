use fil_actor_market::{DealSettlementSummary, EX_DEAL_EXPIRED};
use fil_actors_runtime::EPOCHS_IN_DAY;
use fvm_shared::{clock::ChainEpoch, econ::TokenAmount};

mod harness;
use harness::*;
use num_traits::Zero;

#[test]
fn deal_is_terminated() {
    struct Case {
        name: &'static str,
        deal_start: ChainEpoch,
        deal_end: ChainEpoch,
        activation_epoch: ChainEpoch,
        termination_epoch: ChainEpoch,
        termination_payment: TokenAmount,
    }

    let cases = [
        Case {
            name: "deal is terminated after the startepoch and then settle payments before the endepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 15,
            termination_payment: TokenAmount::from_atto(50), // (15 - 10) * 10 as deal storage fee is 10 per epoch
        },
        Case {
            name: "deal is terminated after the startepoch and then settle payments after the endepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 15,
            termination_payment: TokenAmount::from_atto(50), // (15 - 10) * 10 as deal storage fee is 10 per epoch
        },
        Case {
            name: "deal is terminated at the startepoch and then settle payments before the endepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 10,
            termination_payment: TokenAmount::zero(), // (10 - 10) * 10
        },
        Case {
            name: "deal is terminated at the startepoch and then settle payments after the endepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 10,
            termination_payment: TokenAmount::zero(), // (10 - 10) * 10
        },
        Case {
            name: "deal is terminated at the activationepoch and then settle payments before the startepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 5,
            termination_payment: TokenAmount::zero(), // (10 - 10) * 10
        },
        Case {
            name: "deal is terminated at the activationepoch and then settle payments after the startepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 5,
            termination_payment: TokenAmount::zero(), // (10 - 10) * 10
        },
        Case {
            name: "deal is terminated at the activationepoch and then settle payments after the endepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 5,
            termination_payment: TokenAmount::zero(), // (10 - 10) * 10
        },
    ];

    for tc in cases {
        eprintln!("running test case: {}", tc.name);
        let rt = setup();
        let sector_number = 7;
        // publish and activate
        rt.set_epoch(tc.activation_epoch);
        let (deal_id, deal_proposal) = publish_and_activate_deal(
            &rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            sector_number,
            tc.deal_start,
            tc.deal_end,
            tc.activation_epoch,
            tc.deal_end,
        );

        // terminate
        rt.set_epoch(tc.termination_epoch);
        let (pay, slashed) = terminate_deals_and_assert_balances(
            &rt,
            CLIENT_ADDR,
            PROVIDER_ADDR,
            &[sector_number],
            &[deal_id],
        );

        assert_eq!(tc.termination_payment, pay);
        assert_eq!(deal_proposal.provider_collateral, slashed);

        assert_deal_deleted(&rt, deal_id, &deal_proposal, sector_number);

        // assert that trying to settle is always a no-op after termination

        // immediately after termination
        settle_deal_payments_no_change(&rt, PROVIDER_ADDR, CLIENT_ADDR, PROVIDER_ADDR, &[deal_id]);
        let mut epoch = tc.termination_epoch + 1;
        rt.set_epoch(epoch);

        // at deal start (if deal was terminated before start)
        if epoch < tc.deal_start {
            epoch = tc.deal_start;
            rt.set_epoch(epoch);
            settle_deal_payments_no_change(
                &rt,
                PROVIDER_ADDR,
                CLIENT_ADDR,
                PROVIDER_ADDR,
                &[deal_id],
            );
        }

        // during deal (if deal was terminated before end)
        if epoch < tc.deal_end {
            epoch = tc.deal_end;
            rt.set_epoch(epoch);
            settle_deal_payments_no_change(
                &rt,
                PROVIDER_ADDR,
                CLIENT_ADDR,
                PROVIDER_ADDR,
                &[deal_id],
            );
        }

        if epoch < tc.deal_end + 1 {
            epoch = tc.deal_end + 1;
            rt.set_epoch(epoch);
            settle_deal_payments_no_change(
                &rt,
                PROVIDER_ADDR,
                CLIENT_ADDR,
                PROVIDER_ADDR,
                &[deal_id],
            );
        }

        check_state(&rt);
    }
}

#[test]
fn settle_payments_then_terminate_deal_in_the_same_epoch() {
    let start_epoch = ChainEpoch::from(50);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let termination_epoch = start_epoch + 100;
    let sector_number = 7;
    let sector_expiry = end_epoch + 100;
    let deal_duration = termination_epoch - start_epoch;

    let rt = setup();

    let (deal_id, proposal) = publish_and_activate_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_number,
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    let client_before = get_balance(&rt, &CLIENT_ADDR);
    let provider_before = get_balance(&rt, &PROVIDER_ADDR);

    // settle payments then terminate
    rt.set_epoch(termination_epoch);
    let expected_payment = deal_duration * &proposal.storage_price_per_epoch;
    let ret = settle_deal_payments(&rt, PROVIDER_ADDR, &[deal_id], &[], &[]);
    assert_eq!(
        &ret.settlements[0],
        &DealSettlementSummary { completed: false, payment: expected_payment.clone() }
    );
    terminate_deals_and_assert_balances(
        &rt,
        CLIENT_ADDR,
        PROVIDER_ADDR,
        &[sector_number],
        &[deal_id],
    );
    assert_deal_deleted(&rt, deal_id, &proposal, sector_number);

    // end state should be equivalent to only calling termination
    let client_after = get_balance(&rt, &CLIENT_ADDR);
    let provider_after = get_balance(&rt, &PROVIDER_ADDR);
    let expected_slash = proposal.provider_collateral;
    assert_eq!(&client_after.balance, &(client_before.balance - &expected_payment));
    assert!(&client_after.locked.is_zero());
    assert_eq!(
        &provider_after.balance,
        &(provider_before.balance + &expected_payment - expected_slash)
    );
    assert!(&provider_after.locked.is_zero());

    check_state(&rt);
}

#[test]
fn terminate_a_deal_then_settle_it_in_the_same_epoch() {
    let start_epoch = ChainEpoch::from(50);
    let end_epoch = start_epoch + 200 * EPOCHS_IN_DAY;
    let termination_epoch = start_epoch + 100;
    let sector_expiry = end_epoch + 100;
    let sector_number = 7;
    let rt = setup();

    let (deal_id, proposal) = publish_and_activate_deal(
        &rt,
        CLIENT_ADDR,
        &MinerAddresses::default(),
        sector_number,
        start_epoch,
        end_epoch,
        0,
        sector_expiry,
    );

    // terminate then attempt to settle payment
    rt.set_epoch(termination_epoch);
    terminate_deals_and_assert_balances(
        &rt,
        CLIENT_ADDR,
        PROVIDER_ADDR,
        &[sector_number],
        &[deal_id],
    );
    let ret = settle_deal_payments(&rt, PROVIDER_ADDR, &[deal_id], &[], &[]);
    assert_eq!(ret.results.codes(), vec![EX_DEAL_EXPIRED]);
    assert_deal_deleted(&rt, deal_id, &proposal, sector_number);

    check_state(&rt);
}
