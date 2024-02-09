// Copyright 2021-2023 Protocol Labs
// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fvm_ipld_encoding::tuple::*;
use fvm_shared::bigint::{bigint_ser, BigInt, Integer};
use fvm_shared::clock::ChainEpoch;

use crate::reward::math::PRECISION;

#[derive(Default, Serialize_tuple, Deserialize_tuple, Clone, Debug, PartialEq, Eq)]
pub struct FilterEstimate {
    #[serde(with = "bigint_ser")]
    pub position: BigInt,
    #[serde(with = "bigint_ser")]
    pub velocity: BigInt,
}

impl FilterEstimate {
    /// Create a new filter estimate given two Q.0 format ints.
    pub fn new(position: BigInt, velocity: BigInt) -> Self {
        FilterEstimate { position: position << PRECISION, velocity: velocity << PRECISION }
    }

    /// Returns the Q.0 position estimate of the filter
    pub fn estimate(&self) -> BigInt {
        &self.position >> PRECISION
    }

    /// Extrapolate filter "position" delta epochs in the future.
    pub fn extrapolate(&self, delta: ChainEpoch) -> BigInt {
        let delta_t = BigInt::from(delta) << PRECISION;
        let position = &self.position << PRECISION;
        (&self.velocity * delta_t) + position
    }
}

pub struct AlphaBetaFilter<'a, 'b, 'f> {
    alpha: &'a BigInt,
    beta: &'b BigInt,
    prev_est: &'f FilterEstimate,
}

impl<'a, 'b, 'f> AlphaBetaFilter<'a, 'b, 'f> {
    pub fn load(prev_est: &'f FilterEstimate, alpha: &'a BigInt, beta: &'b BigInt) -> Self {
        Self { alpha, beta, prev_est }
    }

    pub fn next_estimate(&self, obs: &BigInt, epoch_delta: ChainEpoch) -> FilterEstimate {
        let delta_t = BigInt::from(epoch_delta) << PRECISION;
        let delta_x = (&delta_t * &self.prev_est.velocity) >> PRECISION;
        let mut position = delta_x + &self.prev_est.position;

        let obs = obs << PRECISION;
        let residual = obs - &position;
        let revision_x = (self.alpha * &residual) >> PRECISION;
        position += &revision_x;

        let revision_v = residual * self.beta;
        let revision_v = revision_v.div_floor(&delta_t);
        let velocity = revision_v + &self.prev_est.velocity;
        FilterEstimate { position, velocity }
    }
}

#[cfg(test)]
mod tests {
    use fvm_shared::econ::TokenAmount;
    use fvm_shared::sector::StoragePower;
    use num_traits::Signed;

    use crate::reward::math;
    use crate::EPOCHS_IN_DAY;

    use super::super::smooth_func::{
        extrapolated_cum_sum_of_ratio as ecsor, ln, DEFAULT_ALPHA, DEFAULT_BETA,
    };
    use super::*;

    const ERR_BOUND: u64 = 350;

    // Millionths of difference between val1 and val2
    // (val1 - val2) / val1 * 1e6
    // all inputs Q.128, output Q.0
    fn per_million_error(val_1: &BigInt, val_2: &BigInt) -> BigInt {
        let diff = (val_1 - val_2) << PRECISION;

        let ratio = diff.div_floor(val_1);
        let million = BigInt::from(1_000_000) << PRECISION;

        let diff_per_million = (ratio * million).abs();

        diff_per_million >> (2 * PRECISION)
    }

    fn iterative_cum_sum_of_ratio(
        num: &FilterEstimate,
        denom: &FilterEstimate,
        t0: ChainEpoch,
        delta: ChainEpoch,
    ) -> BigInt {
        let mut ratio = BigInt::from(0u8);

        for i in 0..delta {
            let num_epsilon = num.extrapolate(t0 + i); // Q.256
            let denom_epsilon = denom.extrapolate(t0 + i) >> PRECISION; // Q.256
            let mut epsilon = num_epsilon.div_floor(&denom_epsilon); // Q.256 / Q.128 => Q.128

            if i != 0 && i != delta - 1 {
                epsilon *= 2; // Q.128 * Q.0 => Q.128
            }
            ratio += epsilon;
        }

        ratio.div_floor(&BigInt::from(2))
    }

    fn assert_err_bound(
        num: &FilterEstimate,
        denom: &FilterEstimate,
        delta: ChainEpoch,
        t0: ChainEpoch,
        err_bound: BigInt,
    ) {
        let analytic = ecsor(delta, t0, num, denom);
        let iterative = iterative_cum_sum_of_ratio(num, denom, t0, delta);
        let actual_err = per_million_error(&analytic, &iterative);
        assert!(actual_err < err_bound, "Values are {} and {}", actual_err, err_bound);
    }

    // Returns an estimate with position val and velocity 0
    fn testing_constant_estimate(val: BigInt) -> FilterEstimate {
        FilterEstimate::new(val, BigInt::from(0u8))
    }

    // Returns and estimate with postion x and velocity v
    fn testing_estimate(x: BigInt, v: BigInt) -> FilterEstimate {
        FilterEstimate::new(x, v)
    }

    #[test]
    fn test_natural_log() {
        let ln_inputs: Vec<BigInt> = math::poly_parse(&[
            "340282366920938463463374607431768211456", // Q.128 format of 1
            "924990000000000000000000000000000000000", // Q.128 format of e (rounded up in 5th decimal place to handle truncation)
            "34028236692093846346337460743176821145600000000000000000000", // Q.128 format of 100e18
            "6805647338418769269267492148635364229120000000000000000000000", // Q.128 format of 2e22
            "204169000000000000000000000000000000",    // Q.128 format of 0.0006
            "34028236692093846346337460743",           // Q.128 format of 1e-10
        ])
        .unwrap();

        let expected_ln_outputs: Vec<BigInt> = math::poly_parse(&[
            "0",                                         // Q.128 format of 0 = ln(1)
            "340282366920938463463374607431768211456",   // Q.128 format of 1 = ln(e)
            "15670582109617661336106769654068947397831", // Q.128 format of 46.051... = ln(100e18)
            "17473506083804940763855390762239996622013", // Q.128 format of  51.35... = ln(2e22)
            "-2524410000000000000000000000000000000000", // Q.128 format of -7.41.. = ln(0.0006)
            "-7835291054808830668053384827034473698915", // Q.128 format of -23.02.. = ln(1e-10)
        ])
        .unwrap();

        assert_eq!(ln_inputs.len(), expected_ln_outputs.len());
        let num_inputs = ln_inputs.len();

        for i in 0..num_inputs {
            let z = &ln_inputs[i];
            let ln_of_z = ln(z);
            let expected_z = &expected_ln_outputs[i];
            assert_eq!(expected_z >> PRECISION, ln_of_z >> PRECISION);
        }
    }

    #[test]
    fn constant_estimate() {
        let num_estimate = testing_constant_estimate(BigInt::from(4_000_000));
        let denom_estimate = testing_constant_estimate(BigInt::from(1));

        // 4e6/1 over 1000 epochs should give us 4e9
        let csr_1 = ecsor(1000, 0, &num_estimate, &denom_estimate) >> PRECISION;
        assert_eq!(BigInt::from(4 * 10_i64.pow(9)), csr_1);

        // if we change t0 nothing should change because velocity is 0
        let csr_2 = ecsor(1000, 10_i64.pow(15), &num_estimate, &denom_estimate) >> PRECISION;

        assert_eq!(csr_1, csr_2);

        // 1e12 / 200e12 for 100 epochs should give ratio of 1/2
        let num_estimate = testing_constant_estimate(BigInt::from(10_i64.pow(12)));
        let denom_estimate = testing_constant_estimate(BigInt::from(200 * 10_i64.pow(12)));
        let csr_frac = ecsor(100, 0, &num_estimate, &denom_estimate);

        // If we didn't return Q.128 we'd just get zero
        assert_eq!(BigInt::from(0u8), &csr_frac >> PRECISION);

        // multiply by 10k and we'll get 5k
        // note: this is a bit sensative to input, lots of numbers approach from below
        // (...99999) and so truncating division takes us off by one
        let product = csr_frac * (BigInt::from(10_000) << PRECISION); // Q.256
        assert_eq!(BigInt::from(5000), product >> (2 * PRECISION));
    }

    #[test]
    fn both_positive_velocity() {
        let num_estimate = testing_estimate(BigInt::from(111), BigInt::from(12));
        let denom_estimate = testing_estimate(BigInt::from(3456), BigInt::from(8));
        assert_err_bound(&num_estimate, &denom_estimate, 10_000, 0, BigInt::from(ERR_BOUND));
    }

    #[test]
    fn flipped_signs() {
        let num_estimate = testing_estimate(BigInt::from(1_000_000), BigInt::from(-100));
        let denom_estimate = testing_estimate(BigInt::from(70_000), BigInt::from(1000));
        assert_err_bound(&num_estimate, &denom_estimate, 100_000, 0, BigInt::from(ERR_BOUND));
    }

    #[test]
    fn values_in_range() {
        let tens_of_fil = TokenAmount::from_whole(50).atto().clone();
        let one_fil_per_sec = BigInt::from(25);
        let four_fil_per_second = BigInt::from(100);

        let slow_money = testing_estimate(tens_of_fil.clone(), one_fil_per_sec);
        let fast_money = testing_estimate(tens_of_fil, four_fil_per_second);

        let tens_of_ei_bs = StoragePower::from(10_i128.pow(19));
        let thousands_of_ei_bs = StoragePower::from(2 * 10_i128.pow(22));

        let one_byte_per_epoch_velocity = BigInt::from(1);
        let ten_pi_bs_per_day_velocity =
            BigInt::from(10 * 2_i128.pow(50)) / BigInt::from(EPOCHS_IN_DAY);
        let one_ei_bs_per_day_velocity = BigInt::from(2_i128.pow(60)) / BigInt::from(EPOCHS_IN_DAY);

        let delta = EPOCHS_IN_DAY;
        let t0 = 0;
        let err_bound = BigInt::from(ERR_BOUND);

        let test_cases: Vec<(StoragePower, BigInt)> = vec![
            (tens_of_ei_bs.clone(), one_byte_per_epoch_velocity.clone()),
            (tens_of_ei_bs.clone(), ten_pi_bs_per_day_velocity.clone()),
            (tens_of_ei_bs, one_ei_bs_per_day_velocity.clone()),
            (thousands_of_ei_bs.clone(), one_byte_per_epoch_velocity),
            (thousands_of_ei_bs.clone(), ten_pi_bs_per_day_velocity),
            (thousands_of_ei_bs, one_ei_bs_per_day_velocity),
        ];

        for test_case in test_cases {
            let power = testing_estimate(test_case.0, test_case.1);
            assert_err_bound(&slow_money, &power, delta, t0, err_bound.clone());
            assert_err_bound(&fast_money, &power, delta, t0, err_bound.clone());
        }
    }

    #[test]
    fn rounding() {
        // Calculations in this mod are under the assumption division is euclidean and not truncated
        let dd: BigInt = BigInt::from(-100);
        let dv: BigInt = BigInt::from(3);
        assert_eq!(dd.div_floor(&dv), BigInt::from(-34));

        let dd: BigInt = BigInt::from(200);
        let dv: BigInt = BigInt::from(3);
        assert_eq!(dd.div_floor(&dv), BigInt::from(66));
    }

    #[test]
    fn rounding_issue() {
        let fe = FilterEstimate {
            position: "12340768897043811082913117521041414330876498465539749838848"
                .parse()
                .unwrap(),
            velocity: "-37396269384748225153347462373739139597454335279104".parse().unwrap(),
        };
        let filter_reward = AlphaBetaFilter::load(&fe, &DEFAULT_ALPHA, &DEFAULT_BETA);
        let next = filter_reward.next_estimate(&36266252337034982540u128.into(), 3);
        assert_eq!(
            next.position.to_string(),
            "12340768782449774548722755900999027209659079673176744001536"
        );
        assert_eq!(
            next.velocity.to_string(),
            "-37396515542149801792802995707072472930787668612438"
        );
    }
}
