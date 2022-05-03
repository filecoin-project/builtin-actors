// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::network::EPOCHS_IN_DAY;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

use num_traits::Zero;

mod harness;
use harness::*;

const SECTOR_EXPIRY: ChainEpoch = 400 + 200 * EPOCHS_IN_DAY;

#[test]
fn deal_is_slashed() {
    struct Case {
        name: &'static str,
        deal_start: ChainEpoch,
        deal_end: ChainEpoch,
        activation_epoch: ChainEpoch,
        termination_epoch: ChainEpoch,
        payment: TokenAmount,
    }

    let cases = [
        Case {
            name: "deal is slashed after the startepoch and then the first crontick happens",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 15,
            payment: TokenAmount::from(50), // (15 - 10) * 10 as deal storage fee is 10 per epoch
        },
        Case {
            name: "deal is slashed at the startepoch and then the first crontick happens",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 10,
            payment: TokenAmount::from(0), // (10 - 10) * 10
        },
        Case {
            name: "deal is slashed before the startepoch and then the first crontick happens",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 6,
            payment: TokenAmount::from(0), // (10 - 10) * 10
        },
        Case {
            name: "deal is terminated at the activation epoch and then the first crontick happens",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 5,
            payment: TokenAmount::from(0), // (10 - 10) * 10
        },
        Case {
            name: "deal is slashed and then deal expiry happens on crontick, but slashing still occurs",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 15,
            payment: TokenAmount::from(50),
        },
        Case {
            name: "deal is slashed just BEFORE the end epoch",
            deal_start: 10,
            deal_end: 10 + 200 * EPOCHS_IN_DAY,
            activation_epoch: 5,
            termination_epoch: 19,
            payment: TokenAmount::from(90), // (19 - 10) * 10
        }];
    for tc in cases {
        eprintln!("Running testcase: {}", tc.name);
        let mut rt = setup();

        // publish and activate
        rt.set_epoch(tc.activation_epoch);
        let deal_id = publish_and_activate_deal(
            &mut rt,
            CLIENT_ADDR,
            &MinerAddresses::default(),
            tc.deal_start,
            tc.deal_end,
            tc.activation_epoch,
            SECTOR_EXPIRY,
        );
        let deal_proposal = get_deal_proposal(&mut rt, deal_id);

        // terminate
        rt.set_epoch(tc.termination_epoch);
        terminate_deals(&mut rt, PROVIDER_ADDR, &[deal_id]);

        // cron tick
        let cron_tick_epoch = process_epoch(tc.deal_start, deal_id);
        rt.set_epoch(cron_tick_epoch);

        let (pay, slashed) = cron_tick_and_assert_balances(
            &mut rt,
            CLIENT_ADDR,
            PROVIDER_ADDR,
            cron_tick_epoch,
            deal_id,
        );
        assert_eq!(tc.payment, pay);
        assert_eq!(deal_proposal.provider_collateral, slashed);
        assert_deal_deleted(&mut rt, deal_id, deal_proposal);

        // if there has been no payment, provider will have zero balance and hence should be slashed
        if tc.payment.is_zero() {
            assert_account_zero(&mut rt, PROVIDER_ADDR);
            // client balances should not change
            let c_locked = get_locked_balance(&mut rt, CLIENT_ADDR);
            let c_escrow = get_escrow_balance(&rt, &CLIENT_ADDR).unwrap();
            cron_tick(&mut rt);
            assert_eq!(c_escrow, get_escrow_balance(&rt, &CLIENT_ADDR).unwrap());
            assert_eq!(c_locked, get_locked_balance(&mut rt, CLIENT_ADDR));
        } else {
            // running cron tick again dosen't do anything
            cron_tick_no_change(&mut rt, CLIENT_ADDR, PROVIDER_ADDR);
        }
        check_state(&rt);
    }
}
