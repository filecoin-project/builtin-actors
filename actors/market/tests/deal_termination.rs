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
        settlement_epoch: ChainEpoch,
        payment: TokenAmount,
    }

    let cases = [
        Case {
            name: "deal is terminated after the startepoch and then settle payments before the endepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 15,
            settlement_epoch: 20,
            payment: TokenAmount::from_atto(50), // (15 - 10) * 10 as deal storage fee is 10 per epoch
        },
        Case {
            name: "deal is terminated after the startepoch and then settle payments after the endepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 15,
            settlement_epoch: 10 + 200 * EPOCHS_IN_DAY + 10,
            payment: TokenAmount::from_atto(50), // (15 - 10) * 10 as deal storage fee is 10 per epoch
        },
        Case {
            name: "deal is terminated at the startepoch and then settle payments before the endepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 10,
            settlement_epoch: 20,
            payment: TokenAmount::zero(), // (10 - 10) * 10
        },
        Case {
            name: "deal is terminated at the startepoch and then settle payments after the endepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 10,
            settlement_epoch: 10 + 200 * EPOCHS_IN_DAY + 10,
            payment: TokenAmount::zero(), // (10 - 10) * 10
        },
        Case {
            name: "deal is terminated at the activationepoch and then settle payments before the startepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 5,
            settlement_epoch: 9,
            payment: TokenAmount::zero(), // (10 - 10) * 10
        },
        Case {
            name: "deal is terminated at the activationepoch and then settle payments after the startepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 5,
            settlement_epoch: 20,
            payment: TokenAmount::zero(), // (10 - 10) * 10
        },
        Case {
            name: "deal is terminated at the activationepoch and then settle payments after the endepoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 5,
            settlement_epoch: 10 + 200 * EPOCHS_IN_DAY + 10,
            payment: TokenAmount::zero(), // (10 - 10) * 10
        },
    ];

    for tc in cases {
        eprintln!("running test case: {}", tc.name);
        let rt = setup();

        // publish and activate
        rt.set_epoch(tc.activation_epoch);
        let (deal_id, deal_proposal) = publish_and_activate_deal(
            &rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            tc.deal_start,
            tc.deal_end,
            tc.activation_epoch,
            tc.deal_end,
        );

        // terminate
        rt.set_epoch(tc.termination_epoch);
        let (pay, slashed) =
            terminate_deals_and_assert_balances(&rt, CLIENT_ADDR, PROVIDER_ADDR, &[deal_id]);

        assert_eq!(tc.payment, pay);
        assert_eq!(deal_proposal.provider_collateral, slashed);

        // go to settlement epoch
        rt.set_epoch(tc.settlement_epoch);
        // assert that trying to settle is always a no-op after termination
        settle_deal_payments_no_change(&rt, PROVIDER_ADDR, CLIENT_ADDR, PROVIDER_ADDR, &[deal_id]);

        assert_deal_deleted(&rt, deal_id, &deal_proposal);
        check_state(&rt);
    }
}
