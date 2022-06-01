use fil_actor_miner::quality_for_weight;
use fil_actor_miner::{
    DEAL_WEIGHT_MULTIPLIER, QUALITY_BASE_MULTIPLIER, SECTOR_QUALITY_PRECISION,
    VERIFIED_DEAL_WEIGHT_MULTIPLIER,
};
use fil_actors_runtime::EPOCHS_IN_DAY;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::SectorSize;

#[test]
fn quality_is_independent_of_size_and_duration() {
    // Quality of space with no deals. This doesn't depend on either the sector size or duration.
    let empty_quality = TokenAmount::from(1 << SECTOR_QUALITY_PRECISION);
    // Quality space filled with non-verified deals.
    let deal_quality =
        &empty_quality * (DEAL_WEIGHT_MULTIPLIER.clone() / QUALITY_BASE_MULTIPLIER.clone());
    // Quality space filled with verified deals.
    let verified_quality = &empty_quality
        * (VERIFIED_DEAL_WEIGHT_MULTIPLIER.clone() / QUALITY_BASE_MULTIPLIER.clone());

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
            let sector_weight = weight(size, *duration);
            assert_eq!(
                empty_quality,
                quality_for_weight(size, *duration, &BigInt::from(0), &BigInt::from(0)),
            );
            assert_eq!(
                deal_quality,
                quality_for_weight(size, *duration, &sector_weight, &BigInt::from(0)),
            );
            assert_eq!(
                verified_quality,
                quality_for_weight(size, *duration, &BigInt::from(0), &sector_weight),
            );
        }
    }
}

#[test]
fn quality_scales_with_verified_weight_proportion() {
    // Quality of space with no deals. This doesn't depend on either the sector size or duration.
    let empty_quality = TokenAmount::from(1 << SECTOR_QUALITY_PRECISION);
    // Quality space filled with verified deals.
    let verified_quality = &empty_quality
        * (VERIFIED_DEAL_WEIGHT_MULTIPLIER.clone() / QUALITY_BASE_MULTIPLIER.clone());

    let sector_size = SectorSize::_64GiB;
    let sector_duration = ChainEpoch::from(1_000_000); // ~350 days
    let sector_weight = weight(sector_size, sector_duration);

    let verified_range: Vec<BigInt> = vec![
        BigInt::from(0),
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
            quality_for_weight(sector_size, sector_duration, &BigInt::from(0), &verified_weight),
        );
    }
}

fn weight(size: SectorSize, duration: ChainEpoch) -> BigInt {
    BigInt::from(size as u64) * BigInt::from(duration as i64)
}

fn weight_with_size_as_bigint(size: BigInt, duration: ChainEpoch) -> BigInt {
    size * BigInt::from(duration as i64)
}
