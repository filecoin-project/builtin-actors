use fil_actor_market::SectorWeights;
use fil_actor_miner::PreCommitSectorBatchParams;
use fil_actor_miner::VestSpec;
use fil_actor_miner::{
    initial_pledge_for_power, pre_commit_deposit_for_power, qa_power_for_weight, PowerPair,
};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::DealWeight;
use fvm_shared::bigint::Zero;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
use fvm_shared::sector::MAX_SECTOR_NUMBER;
use std::collections::HashMap;

mod util;
use util::*;

struct TestCase {
    desc: &'static str,
    vspec: VestSpec,
    period_start: ChainEpoch,
    vepocs: Vec<i64>,
}

#[test]
fn test_vesting() {
    let vest_start_delay = 10;
    let vest_sum = 100;

    let tests = [
        TestCase {
            desc: "vest funds in a single epoch",
            vspec: VestSpec { initial_delay: 0, vest_period: 1, step_duration: 1, quantization: 1 },
            vepocs: vec![0, 0, 100, 0],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with period=2",
            vspec: VestSpec { initial_delay: 0, vest_period: 2, step_duration: 1, quantization: 1 },
            vepocs: vec![0, 0, 50, 50, 0],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with period=2 quantization=2",
            vspec: VestSpec { initial_delay: 0, vest_period: 2, step_duration: 1, quantization: 2 },
            vepocs: vec![0, 0, 0, 100, 0],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with period=3",
            vspec: VestSpec { initial_delay: 0, vest_period: 3, step_duration: 1, quantization: 1 },
            vepocs: vec![0, 0, 33, 33, 34, 0],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with period=3 quantization=2",
            vspec: VestSpec { initial_delay: 0, vest_period: 3, step_duration: 1, quantization: 2 },
            vepocs: vec![0, 0, 0, 66, 0, 34, 0],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with period=2 step=2",
            vspec: VestSpec { initial_delay: 0, vest_period: 2, step_duration: 2, quantization: 1 },
            vepocs: vec![0, 0, 0, 100, 0],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with period=5 step=2",
            vspec: VestSpec { initial_delay: 0, vest_period: 5, step_duration: 2, quantization: 1 },
            vepocs: vec![0, 0, 0, 40, 0, 40, 0, 20, 0],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with delay=1 period=5 step=2",
            vspec: VestSpec { initial_delay: 1, vest_period: 5, step_duration: 2, quantization: 1 },
            vepocs: vec![0, 0, 0, 0, 40, 0, 40, 0, 20, 0],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with period=5 step=2 quantization=2",
            vspec: VestSpec { initial_delay: 0, vest_period: 5, step_duration: 2, quantization: 2 },
            vepocs: vec![0, 0, 0, 40, 0, 40, 0, 20, 0],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with period=5 step=3 quantization=1",
            vspec: VestSpec { initial_delay: 0, vest_period: 5, step_duration: 3, quantization: 1 },
            vepocs: vec![0, 0, 0, 0, 60, 0, 0, 40, 0],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with period=5 step=3 quantization=2",
            vspec: VestSpec { initial_delay: 0, vest_period: 5, step_duration: 3, quantization: 2 },
            vepocs: vec![0, 0, 0, 0, 0, 80, 0, 20, 0],
            period_start: 0,
        },
        TestCase {
            desc: "(step greater than period) vest funds with period=5 step=6 quantization=1",
            vspec: VestSpec { initial_delay: 0, vest_period: 5, step_duration: 6, quantization: 1 },
            vepocs: vec![0, 0, 0, 0, 0, 0, 0, 100, 0],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with delay=5 period=5 step=1 quantization=1",
            vspec: VestSpec { initial_delay: 5, vest_period: 5, step_duration: 1, quantization: 1 },
            vepocs: vec![0, 0, 0, 0, 0, 0, 0, 20, 20, 20, 20, 20, 0],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with offset 0",
            vspec: VestSpec {
                initial_delay: 0,
                vest_period: 10,
                step_duration: 2,
                quantization: 2,
            },
            vepocs: vec![0, 0, 0, 20, 0, 20, 0, 20, 0, 20, 0, 20],
            period_start: 0,
        },
        TestCase {
            desc: "vest funds with offset 1",
            vspec: VestSpec {
                initial_delay: 0,
                vest_period: 10,
                step_duration: 2,
                quantization: 2,
            },
            period_start: 1,
            // start epoch is at 11 instead of 10 so vepocs are shifted by one from above case
            vepocs: vec![0, 0, 0, 20, 0, 20, 0, 20, 0, 20, 0, 20],
            // vepocs: vec![0, 0, 0, 0, 30, 0, 20, 0, 20, 0, 20, 0, 10],
        },
        TestCase {
            desc: "vest funds with proving period start > quantization unit",
            vspec: VestSpec {
                initial_delay: 0,
                vest_period: 10,
                step_duration: 2,
                quantization: 2,
            },
            // 55 % 2 = 1 so expect same vepocs with offset 1 as in previous case
            period_start: 55,
            vepocs: vec![0, 0, 0, 20, 0, 20, 0, 20, 0, 20, 0, 20],
        },
        TestCase {
            desc: "vest funds with step much smaller than quantization",
            vspec: VestSpec {
                initial_delay: 0,
                vest_period: 10,
                step_duration: 1,
                quantization: 5,
            },
            vepocs: vec![0, 0, 0, 0, 0, 0, 50, 0, 0, 0, 0, 50],
            period_start: 0,
        },
    ];
    for (nth, tc) in tests.iter().enumerate() {
        let h = ActorHarness::new(tc.period_start);
        let mut rt = h.new_runtime();
        h.construct_and_verify(&mut rt);
        rt.set_epoch(tc.period_start);

        let vest_start = tc.period_start + vest_start_delay;

        let mut st = h.get_state(&rt);
        st.add_locked_funds(&rt.store, vest_start, &TokenAmount::from(vest_sum), &tc.vspec)
            .unwrap();
        assert_eq!(TokenAmount::from(vest_sum), st.locked_funds);

        let mut total_vested = 0;
        for (e, &v) in tc.vepocs.iter().enumerate() {
            dbg!(nth, tc.desc, e, v);
            assert_eq!(
                TokenAmount::from(v),
                st.unlock_vested_funds(&rt.store, vest_start + e as ChainEpoch).unwrap()
            );
        }
    }
}
/*

    }
    for _, tc := range testcase {
        t.Run(tc.desc, func(t *testing.T) {
            harness := constructStateHarness(t, tc.periodStart)
            vestStart := tc.periodStart + vestStartDelay

            harness.addLockedFunds(vestStart, abi.NewTokenAmount(vestSum), tc.vspec)
            assert.Equal(t, abi.NewTokenAmount(vestSum), harness.s.LockedFunds)

            var totalVested int64
            for e, v := range tc.vepocs {
                assert.Equal(t, abi.NewTokenAmount(v), harness.unlockVestedFunds(vestStart+abi.ChainEpoch(e)))
                totalVested += v
                assert.Equal(t, vestSum-totalVested, harness.s.LockedFunds.Int64())
            }

            assert.Equal(t, abi.NewTokenAmount(vestSum), abi.NewTokenAmount(totalVested))
            assert.True(t, harness.vestingFundsStoreEmpty())
            assert.Zero(t, harness.s.LockedFunds.Int64())
        })
    }
} */
