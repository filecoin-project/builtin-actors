use fil_actor_miner::{qa_power_for_weight, quality_for_weight};
use fil_actor_miner::{
    QUALITY_BASE_MULTIPLIER, SECTOR_QUALITY_PRECISION, VERIFIED_DEAL_WEIGHT_MULTIPLIER,
};
use fil_actors_runtime::DealWeight;
use fil_actors_runtime::{EPOCHS_IN_DAY, SECONDS_IN_DAY};
use fvm_shared::bigint::{BigInt, Integer, Zero};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::sector::SectorSize;

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
