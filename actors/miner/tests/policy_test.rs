use fil_actor_miner::{
    QUALITY_BASE_MULTIPLIER, SECTOR_QUALITY_PRECISION, VERIFIED_DEAL_WEIGHT_MULTIPLIER,
};
use fil_actor_miner::{daily_proof_fee, qa_power_for_weight, quality_for_weight};
use fil_actor_miner::{qa_power_for_sector, qa_power_max, SectorOnChainInfo, SectorOnChainInfoFlags};
use fil_actors_runtime::DealWeight;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{EPOCHS_IN_DAY, SECONDS_IN_DAY};
use fvm_shared::bigint::{BigInt, Integer, Zero};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::SectorSize;

use num_traits::Signed;

#[test]
fn quality_is_independent_of_size_and_duration() {
    // Quality of space with no deals. This doesn't depend on either the sector size or duration.
    let empty_quality = BigInt::from(1 << SECTOR_QUALITY_PRECISION);
    // Quality space filled with verified deals.
    let verified_quality = &empty_quality
        * (VERIFIED_DEAL_WEIGHT_MULTIPLIER.clone() / QUALITY_BASE_MULTIPLIER.clone());
    // Quality space half filled with verified deals.
    let half_verified_quality =
        &empty_quality / BigInt::from(2) + &verified_quality / BigInt::from(2);

    let size_range: Vec<SectorSize> = vec![
        SectorSize::_2KiB,
        SectorSize::_8MiB,
        SectorSize::_512MiB,
        SectorSize::_32GiB,
        SectorSize::_64GiB,
    ];
    let duration_range: Vec<ChainEpoch> = vec![
        ChainEpoch::from(1),
        ChainEpoch::from(10),
        ChainEpoch::from(1000),
        1000 * EPOCHS_IN_DAY,
    ];
    let zero = &BigInt::zero();

    for size in size_range {
        for duration in &duration_range {
            let full_weight = weight(size, *duration);
            let half_weight = &full_weight.checked_div(&BigInt::from(2)).unwrap();

            assert_eq!(empty_quality, quality_for_weight(size, *duration, zero));
            assert_eq!(verified_quality, quality_for_weight(size, *duration, &full_weight));
            assert_eq!(half_verified_quality, quality_for_weight(size, *duration, half_weight));

            // test against old form that takes a deal_weight argument
            assert_eq!(empty_quality, original_quality_for_weight(size, *duration, zero, zero));
            assert_eq!(
                empty_quality,
                original_quality_for_weight(size, *duration, &full_weight, zero)
            );
            assert_eq!(
                empty_quality,
                original_quality_for_weight(size, *duration, half_weight, zero)
            );
            assert_eq!(
                verified_quality,
                original_quality_for_weight(size, *duration, zero, &full_weight)
            );
            assert_eq!(
                verified_quality,
                original_quality_for_weight(size, *duration, half_weight, &full_weight)
            );
            assert_eq!(
                verified_quality,
                original_quality_for_weight(size, *duration, &full_weight, &full_weight)
            );
            assert_eq!(
                half_verified_quality,
                original_quality_for_weight(size, *duration, zero, half_weight)
            );
            assert_eq!(
                half_verified_quality,
                original_quality_for_weight(size, *duration, half_weight, half_weight)
            );
            assert_eq!(
                half_verified_quality,
                original_quality_for_weight(size, *duration, &full_weight, half_weight)
            );
        }
    }
}

#[test]
fn quality_scales_with_verified_weight_proportion() {
    // Quality of space with no deals. This doesn't depend on either the sector size or duration.
    let empty_quality = BigInt::from(1 << SECTOR_QUALITY_PRECISION);
    // Quality space filled with verified deals.
    let verified_quality = &empty_quality
        * (VERIFIED_DEAL_WEIGHT_MULTIPLIER.clone() / QUALITY_BASE_MULTIPLIER.clone());

    let sector_size = SectorSize::_64GiB;
    let sector_duration = ChainEpoch::from(1_000_000); // ~350 days
    let sector_weight = weight(sector_size, sector_duration);

    let verified_range: Vec<BigInt> = vec![
        BigInt::zero(),
        BigInt::from(1),
        BigInt::from(1 << 10),
        BigInt::from(2 << 20),
        BigInt::from(5 << 20),
        BigInt::from(1 << 30),
        BigInt::from((2i64 << 35) - 1),
        BigInt::from(2i64 << 35),
    ];
    for verified_space in verified_range {
        let verified_weight = weight_with_size_as_bigint(verified_space.clone(), sector_duration);
        let empty_weight =
            weight_with_size_as_bigint(sector_size as u64 - verified_space, sector_duration);
        assert_eq!((sector_weight.clone() - empty_weight.clone()), verified_weight);

        // Expect sector quality to be a weighted sum of base and verified quality
        let eq = empty_weight * empty_quality.clone();
        let vq = &verified_weight * verified_quality.clone();
        let expected_quality = (eq + vq) / &sector_weight;
        assert_eq!(
            expected_quality,
            quality_for_weight(sector_size, sector_duration, &verified_weight),
        );
    }
}

#[test]
fn empty_sector_has_power_equal_to_size() {
    let size_range: Vec<SectorSize> = vec![
        SectorSize::_2KiB,
        SectorSize::_8MiB,
        SectorSize::_512MiB,
        SectorSize::_32GiB,
        SectorSize::_64GiB,
    ];
    let duration_range: Vec<ChainEpoch> = vec![
        ChainEpoch::from(1),
        ChainEpoch::from(10),
        ChainEpoch::from(1000),
        1000 * EPOCHS_IN_DAY,
    ];
    for size in size_range {
        for duration in &duration_range {
            let expected_power = BigInt::from(size as i64);
            assert_eq!(expected_power, qa_power_for_weight(size, *duration, &BigInt::zero()));
        }
    }
}

#[test]
fn verified_sector_has_power_a_multiple_of_size() {
    let verified_multiplier =
        VERIFIED_DEAL_WEIGHT_MULTIPLIER.clone() / QUALITY_BASE_MULTIPLIER.clone();
    let size_range: Vec<SectorSize> = vec![
        SectorSize::_2KiB,
        SectorSize::_8MiB,
        SectorSize::_512MiB,
        SectorSize::_32GiB,
        SectorSize::_64GiB,
    ];
    let duration_range: Vec<ChainEpoch> = vec![
        ChainEpoch::from(1),
        ChainEpoch::from(10),
        ChainEpoch::from(1000),
        1000 * EPOCHS_IN_DAY,
    ];
    for size in size_range {
        for duration in &duration_range {
            let verified_weight = weight(size, *duration);
            let expected_power = size as i64 * &verified_multiplier;
            assert_eq!(expected_power, qa_power_for_weight(size, *duration, &verified_weight));
        }
    }
}

#[test]
fn verified_weight_adds_proportional_power() {
    let sector_size = SectorSize::_64GiB;
    let sector_duration = 180 * SECONDS_IN_DAY;
    let sector_weight = weight(sector_size, sector_duration);

    let fully_empty_power = BigInt::from(sector_size as i64);
    let fully_verified_power = (BigInt::from(sector_size as i64)
        * VERIFIED_DEAL_WEIGHT_MULTIPLIER.clone())
        / QUALITY_BASE_MULTIPLIER.clone();

    let max_error = BigInt::from(1 << SECTOR_QUALITY_PRECISION);

    let verified_range: Vec<BigInt> = vec![
        BigInt::zero(),
        BigInt::from(1),
        BigInt::from(1 << 10),
        BigInt::from(2 << 20),
        BigInt::from(5 << 20),
        BigInt::from(32 << 30),
        BigInt::from((2i64 << 35) - 1),
        BigInt::from(2i64 << 35),
    ];
    let duration_range: Vec<ChainEpoch> = vec![
        ChainEpoch::from(0),
        ChainEpoch::from(1),
        sector_duration / 2,
        sector_duration - 1,
        sector_duration,
    ];
    for verified_space in verified_range {
        for verified_duration in &duration_range {
            let verified_weight =
                weight_with_size_as_bigint(verified_space.clone(), *verified_duration);
            let empty_weight = &sector_weight - &verified_weight;

            // Expect sector power to be a weighted sum of base and verified power.
            let ep = empty_weight * &fully_empty_power;
            let vp = &verified_weight * &fully_verified_power;
            let expected_power = (ep + vp) / &sector_weight;
            let power = qa_power_for_weight(sector_size, sector_duration, &verified_weight);
            let power_error = expected_power - power;
            assert!(power_error <= max_error);
        }
    }
}

#[test]
fn demonstrate_standard_sectors() {
    let sector_duration = 180 * EPOCHS_IN_DAY;
    let vmul = VERIFIED_DEAL_WEIGHT_MULTIPLIER.clone() / QUALITY_BASE_MULTIPLIER.clone();

    // 32 GiB
    let sector_size = SectorSize::_32GiB;
    let sector_weight = weight(sector_size, sector_duration);

    assert_eq!(
        BigInt::from(sector_size as u64),
        qa_power_for_weight(sector_size, sector_duration, &BigInt::zero())
    );
    assert_eq!(
        &vmul * sector_size as u64,
        qa_power_for_weight(sector_size, sector_duration, &sector_weight)
    );
    let half_verified_power = ((sector_size as u64) / 2) + (&vmul * (sector_size as u64) / 2);
    assert_eq!(
        half_verified_power,
        qa_power_for_weight(sector_size, sector_duration, &(sector_weight / 2))
    );

    // 64GiB
    let sector_size = SectorSize::_64GiB;
    let sector_weight = weight(sector_size, sector_duration);

    assert_eq!(
        BigInt::from(sector_size as u64),
        qa_power_for_weight(sector_size, sector_duration, &BigInt::zero())
    );
    assert_eq!(
        &vmul * sector_size as u64,
        qa_power_for_weight(sector_size, sector_duration, &sector_weight)
    );
    let half_verified_power = ((sector_size as u64) / 2) + (&vmul * (sector_size as u64) / 2);
    assert_eq!(
        half_verified_power,
        qa_power_for_weight(sector_size, sector_duration, &(sector_weight / 2))
    );
}

fn weight(size: SectorSize, duration: ChainEpoch) -> BigInt {
    BigInt::from(size as u64) * BigInt::from(duration)
}

fn weight_with_size_as_bigint(size: BigInt, duration: ChainEpoch) -> BigInt {
    size * BigInt::from(duration)
}

// Original form of quality_for_weight prior to removing the deal weight multiplier; retained here
// for testing purposes. Since the deal weight multipler has remained fixed at the same value as
// the quality base multiplier (10) it has never had an effect on the result.
fn original_quality_for_weight(
    size: SectorSize,
    duration: ChainEpoch,
    deal_weight: &DealWeight,
    verified_weight: &DealWeight,
) -> BigInt {
    let sector_space_time = BigInt::from(size as u64) * BigInt::from(duration);
    let total_deal_space_time = deal_weight + verified_weight;

    let weighted_base_space_time =
        (&sector_space_time - total_deal_space_time) * &*QUALITY_BASE_MULTIPLIER;
    let weighted_deal_space_time = deal_weight * BigInt::from(10);
    let weighted_verified_space_time = verified_weight * &*VERIFIED_DEAL_WEIGHT_MULTIPLIER;
    let weighted_sum_space_time =
        weighted_base_space_time + weighted_deal_space_time + weighted_verified_space_time;
    let scaled_up_weighted_sum_space_time: BigInt =
        weighted_sum_space_time << SECTOR_QUALITY_PRECISION;

    scaled_up_weighted_sum_space_time
        .div_floor(&sector_space_time)
        .div_floor(&QUALITY_BASE_MULTIPLIER)
}

#[test]
fn daily_proof_fee_calc() {
    let policy = Policy::default();
    // Given a CS of 680M FIL, 32GiB QAP, a fee multiplier of 5.56e-15 per 32GiB QAP, the daily proof
    // fee should be 3780 nanoFIL.
    //   680M * 5.56e-15 = 0.000003780800 FIL
    //   0.0000037808 * 1e9 = 3780 nanoFIL
    //   0.0000037808 * 1e18 = 3780800000000 attoFIL
    // As a per-byte multiplier we use 1.61817e-25, a close approximation of 5.56e-15 / 32GiB.
    //   680M * 32GiB * 1.61817e-25 = 0.000003780793052776 FIL
    //   0.000003780793052776 * 1e18 = 3780793052776 attoFIL
    let circulating_supply = TokenAmount::from_whole(680_000_000);

    let ref_32gib_fee = 3780793052776_u64;
    [
        (32_u64, ref_32gib_fee),
        (64, ref_32gib_fee * 2),
        (32 * 10, ref_32gib_fee * 10),
        (32 * 5, ref_32gib_fee * 5),
        (64 * 10, ref_32gib_fee * 20),
    ]
    .iter()
    .for_each(|(size, expected_fee)| {
        let power = BigInt::from(*size) << 30; // 32GiB raw QAP
        let fee = daily_proof_fee(&policy, &circulating_supply, &power);
        assert!(
            (fee.atto() - BigInt::from(*expected_fee)).abs() <= BigInt::from(10),
            "size: {}, fee: {}, expected_fee: {} (±10)",
            size,
            fee.atto(),
            expected_fee
        );
    });
}

// --- FULL_QA_POWER flag tests ---

#[test]
fn full_qa_power_flag_gives_10x() {
    // A sector with FULL_QA_POWER and zero deal weights should get qa_power_max (10x raw power).
    let sizes = vec![SectorSize::_2KiB, SectorSize::_32GiB];
    for size in sizes {
        let sector = SectorOnChainInfo {
            sector_number: 1,
            flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER | SectorOnChainInfoFlags::FULL_QA_POWER,
            expiration: 1000,
            power_base_epoch: 0,
            ..Default::default()
        };
        let power = qa_power_for_sector(size, &sector);
        let expected = qa_power_max(size);
        assert_eq!(
            power, expected,
            "FULL_QA_POWER sector of size {:?} should get qa_power_max",
            size
        );
        // Verify it's exactly 10x raw power.
        assert_eq!(expected, BigInt::from(size as u64) * 10);
    }
}

#[test]
fn full_qa_power_ignores_deal_weights() {
    // A sector with FULL_QA_POWER but non-zero verified_deal_weight should still get exactly
    // qa_power_max. The deal weights are irrelevant when the flag is set.
    let size = SectorSize::_32GiB;
    let duration: ChainEpoch = 1000;
    let full_verified_weight = weight(size, duration);

    let sector = SectorOnChainInfo {
        sector_number: 1,
        flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER | SectorOnChainInfoFlags::FULL_QA_POWER,
        expiration: duration,
        power_base_epoch: 0,
        verified_deal_weight: full_verified_weight,
        ..Default::default()
    };
    let power = qa_power_for_sector(size, &sector);
    assert_eq!(
        power,
        qa_power_max(size),
        "FULL_QA_POWER should produce qa_power_max regardless of verified_deal_weight"
    );
}

#[test]
fn full_qa_power_ignores_partial_verified() {
    // A sector with FULL_QA_POWER and partial verified weight (half the sector) should still
    // get qa_power_max.
    let size = SectorSize::_32GiB;
    let duration: ChainEpoch = 2000;
    let half_verified_weight = weight(size, duration) / 2;

    let sector = SectorOnChainInfo {
        sector_number: 1,
        flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER | SectorOnChainInfoFlags::FULL_QA_POWER,
        expiration: duration,
        power_base_epoch: 0,
        verified_deal_weight: half_verified_weight,
        ..Default::default()
    };
    let power = qa_power_for_sector(size, &sector);
    assert_eq!(
        power,
        qa_power_max(size),
        "FULL_QA_POWER should produce qa_power_max even with partial verified weight"
    );
}

#[test]
fn legacy_sector_without_flag_uses_old_formula() {
    // A sector WITHOUT FULL_QA_POWER should still use the old quality_for_weight formula:
    // 1x for CC, 10x for fully verified, proportional for partial.
    let size = SectorSize::_32GiB;
    let duration: ChainEpoch = 1000;
    let full_weight = weight(size, duration);

    // CC sector (no verified weight) -> 1x raw power
    let cc_sector = SectorOnChainInfo {
        sector_number: 1,
        flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER, // no FULL_QA_POWER
        expiration: duration,
        power_base_epoch: 0,
        verified_deal_weight: BigInt::zero(),
        ..Default::default()
    };
    assert_eq!(
        qa_power_for_sector(size, &cc_sector),
        BigInt::from(size as u64),
        "Legacy CC sector should have 1x raw power"
    );

    // Fully verified sector -> 10x raw power
    let verified_sector = SectorOnChainInfo {
        sector_number: 2,
        flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER,
        expiration: duration,
        power_base_epoch: 0,
        verified_deal_weight: full_weight.clone(),
        ..Default::default()
    };
    assert_eq!(
        qa_power_for_sector(size, &verified_sector),
        qa_power_max(size),
        "Legacy fully verified sector should have 10x raw power"
    );

    // Half verified sector -> proportional (midpoint between 1x and 10x)
    let half_verified_sector = SectorOnChainInfo {
        sector_number: 3,
        flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER,
        expiration: duration,
        power_base_epoch: 0,
        verified_deal_weight: full_weight / 2,
        ..Default::default()
    };
    let half_power = qa_power_for_sector(size, &half_verified_sector);
    let expected_half = BigInt::from(size as u64) / 2 + qa_power_max(size) / 2;
    assert_eq!(
        half_power, expected_half,
        "Legacy half-verified sector should have proportional power"
    );
}

#[test]
fn full_qa_power_independent_of_duration() {
    // FULL_QA_POWER power should be the same regardless of sector duration/expiration.
    let size = SectorSize::_32GiB;
    let durations: Vec<ChainEpoch> = vec![1, 100, 1000, 180 * EPOCHS_IN_DAY, 540 * EPOCHS_IN_DAY];
    let expected = qa_power_max(size);

    for duration in durations {
        let sector = SectorOnChainInfo {
            sector_number: 1,
            flags: SectorOnChainInfoFlags::SIMPLE_QA_POWER
                | SectorOnChainInfoFlags::FULL_QA_POWER,
            expiration: duration,
            power_base_epoch: 0,
            ..Default::default()
        };
        assert_eq!(
            qa_power_for_sector(size, &sector),
            expected,
            "FULL_QA_POWER power should be identical for duration {}",
            duration
        );
    }
}
