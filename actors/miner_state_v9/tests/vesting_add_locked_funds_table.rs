use fil_actor_miner_state_v9::VestSpec;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;

mod state_harness;
use state_harness::*;

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
    for tc in tests {
        println!("Testing: {}", tc.desc);
        let mut h = StateHarness::new(tc.period_start);

        let vest_start = tc.period_start + vest_start_delay;

        h.st.add_locked_funds(&h.store, vest_start, &TokenAmount::from_atto(vest_sum), &tc.vspec)
            .unwrap();
        assert_eq!(TokenAmount::from_atto(vest_sum), h.st.locked_funds);

        let mut total_vested = 0;
        for (e, &v) in tc.vepocs.iter().enumerate() {
            assert_eq!(
                TokenAmount::from_atto(v),
                h.st.unlock_vested_funds(&h.store, vest_start + e as ChainEpoch).unwrap()
            );
            total_vested += v;
            assert_eq!(TokenAmount::from_atto(vest_sum - total_vested), h.st.locked_funds);
        }

        assert_eq!(vest_sum, total_vested);
        assert!(h.vesting_funds_store_empty());
        assert!(h.st.locked_funds.is_zero());
    }
}
