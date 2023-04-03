// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::cmp;
use std::ops::{DivAssign, MulAssign};

use cid::{Cid, Version};
use fil_actors_runtime::network::*;
use fil_actors_runtime::runtime::Policy;
use fil_actors_runtime::{DealWeight, EXPECTED_LEADERS_PER_EPOCH};
use fvm_shared::bigint::{BigInt, Integer};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::commcid::{FIL_COMMITMENT_SEALED, POSEIDON_BLS12_381_A1_FC1};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::{
    RegisteredPoStProof, RegisteredSealProof, SectorQuality, SectorSize, StoragePower,
};
use lazy_static::lazy_static;

use super::types::SectorOnChainInfo;
use super::{PowerPair, BASE_REWARD_FOR_DISPUTED_WINDOW_POST};

/// Precision used for making QA power calculations
pub const SECTOR_QUALITY_PRECISION: i64 = 20;

/// The minimum sector lifetime to which the SDM applies.
pub const SECTOR_DURATION_MULTIPLIER_START: ChainEpoch = 540 * EPOCHS_IN_DAY;
/// The sector lifetime at which the SDM maxes out.
pub const SECTOR_DURATION_MULTIPLIER_END: ChainEpoch = 5 * EPOCHS_IN_YEAR;

lazy_static! {
    /// Quality multiplier for committed capacity (no deals) in a sector
    pub static ref QUALITY_BASE_MULTIPLIER: BigInt = BigInt::from(10);

    /// Quality multiplier for unverified deals in a sector
    pub static ref DEAL_WEIGHT_MULTIPLIER: BigInt = BigInt::from(10);

    /// Quality multiplier for verified deals in a sector
    pub static ref VERIFIED_DEAL_WEIGHT_MULTIPLIER: BigInt = BigInt::from(100);

    /// Maximum quality multiplier due to SDM + Verified Deals
    pub static ref MAX_WEIGHT_MULTIPLIER: BigInt = BigInt::from(200);
}

/// The maximum number of partitions that may be required to be loaded in a single invocation,
/// when all the sector infos for the partitions will be loaded.
pub fn load_partitions_sectors_max(policy: &Policy, partition_sector_count: u64) -> u64 {
    cmp::min(policy.addressed_sectors_max / partition_sector_count, policy.addressed_partitions_max)
}

/// Prefix for sealed sector CIDs (CommR).
pub fn is_sealed_sector(c: &Cid) -> bool {
    // TODO: Move FIL_COMMITMENT etc, into a better place
    c.version() == Version::V1
        && c.codec() == FIL_COMMITMENT_SEALED
        && c.hash().code() == POSEIDON_BLS12_381_A1_FC1
        && c.hash().size() == 32
}

/// List of proof types which can be used when creating new miner actors
pub fn can_pre_commit_seal_proof(policy: &Policy, proof: RegisteredSealProof) -> bool {
    policy.valid_pre_commit_proof_type.contains(proof)
}

/// Checks whether a seal proof type is supported for new miners and sectors.
pub fn can_extend_seal_proof_type(_proof: RegisteredSealProof) -> bool {
    true
}

/// Maximum duration to allow for the sealing process for seal algorithms.
/// Dependent on algorithm and sector size
pub fn max_prove_commit_duration(
    policy: &Policy,
    proof: RegisteredSealProof,
) -> Option<ChainEpoch> {
    use RegisteredSealProof::*;
    match proof {
        StackedDRG32GiBV1 | StackedDRG2KiBV1 | StackedDRG8MiBV1 | StackedDRG512MiBV1
        | StackedDRG64GiBV1 => Some(EPOCHS_IN_DAY + policy.pre_commit_challenge_delay),
        StackedDRG32GiBV1P1 | StackedDRG64GiBV1P1 | StackedDRG512MiBV1P1 | StackedDRG8MiBV1P1
        | StackedDRG2KiBV1P1 => Some(30 * EPOCHS_IN_DAY + policy.pre_commit_challenge_delay),
        _ => None,
    }
}

/// Maximum duration to allow for the sealing process for seal algorithms.
/// Dependent on algorithm and sector size
pub fn seal_proof_sector_maximum_lifetime(proof: RegisteredSealProof) -> Option<ChainEpoch> {
    use RegisteredSealProof::*;
    match proof {
        StackedDRG32GiBV1 | StackedDRG2KiBV1 | StackedDRG8MiBV1 | StackedDRG512MiBV1
        | StackedDRG64GiBV1 => Some(EPOCHS_IN_DAY * 540),
        StackedDRG32GiBV1P1 | StackedDRG2KiBV1P1 | StackedDRG8MiBV1P1 | StackedDRG512MiBV1P1
        | StackedDRG64GiBV1P1 => Some(EPOCHS_IN_YEAR * 5),
        _ => None,
    }
}

/// minimum number of epochs past the current epoch a sector may be set to expire
pub const MIN_SECTOR_EXPIRATION: i64 = 180 * EPOCHS_IN_DAY;

/// DealWeight and VerifiedDealWeight are spacetime occupied by regular deals and verified deals in a sector.
/// Sum of DealWeight and VerifiedDealWeight should be less than or equal to total SpaceTime of a sector.
/// Sectors full of VerifiedDeals will have a SectorQuality of VerifiedDealWeightMultiplier/QualityBaseMultiplier.
/// Sectors full of Deals will have a SectorQuality of DealWeightMultiplier/QualityBaseMultiplier.
/// Sectors with neither will have a SectorQuality of QualityBaseMultiplier/QualityBaseMultiplier.
/// SectorQuality of a sector is a weighted average of multipliers based on their proportions.
pub fn quality_for_weight(
    size: SectorSize,
    duration: ChainEpoch,
    deal_weight: &DealWeight,
    verified_weight: &DealWeight,
) -> SectorQuality {
    let sector_space_time = BigInt::from(size as u64) * BigInt::from(duration);
    let total_deal_space_time = deal_weight + verified_weight;

    let weighted_base_space_time =
        (&sector_space_time - total_deal_space_time) * &*QUALITY_BASE_MULTIPLIER;
    let weighted_deal_space_time = deal_weight * &*DEAL_WEIGHT_MULTIPLIER;
    let weighted_verified_space_time = verified_weight * &*VERIFIED_DEAL_WEIGHT_MULTIPLIER;
    let weighted_sum_space_time =
        weighted_base_space_time + weighted_deal_space_time + weighted_verified_space_time;
    let scaled_up_weighted_sum_space_time: SectorQuality =
        weighted_sum_space_time << SECTOR_QUALITY_PRECISION;

    let quality = scaled_up_weighted_sum_space_time
        .div_floor(&sector_space_time)
        .div_floor(&QUALITY_BASE_MULTIPLIER);
    // Apply SDM
    sdm_scale(quality, duration)
}

/// Scale `value` by the sector duration multiplier for `duration`.
pub fn sdm_scale<T>(mut value: T, duration: ChainEpoch) -> T
where
    T: MulAssign<ChainEpoch> + DivAssign<ChainEpoch>,
{
    if duration > SECTOR_DURATION_MULTIPLIER_START {
        if duration > SECTOR_DURATION_MULTIPLIER_END {
            log::error!("sector lifetime {duration} exceeded maximum expected lifetime {SECTOR_DURATION_MULTIPLIER_END}");
            value *= 2;
        } else {
            let numerator = duration - SECTOR_DURATION_MULTIPLIER_START;
            let denominator = SECTOR_DURATION_MULTIPLIER_END - SECTOR_DURATION_MULTIPLIER_START;
            // scale from 1x to 2x.
            value *= (numerator + denominator) * duration;
            value /= denominator;
        }
    }
    value
}

/// Returns maximum achievable QA power.
pub fn qa_power_max(size: SectorSize) -> StoragePower {
    (BigInt::from(size as u64) * &*MAX_WEIGHT_MULTIPLIER).div_floor(&QUALITY_BASE_MULTIPLIER)
}

/// Returns the power for a sector size and weight.
pub fn qa_power_for_weight(
    size: SectorSize,
    duration: ChainEpoch,
    deal_weight: &DealWeight,
    verified_weight: &DealWeight,
) -> StoragePower {
    let quality = quality_for_weight(size, duration, deal_weight, verified_weight);
    (BigInt::from(size as u64) * quality) >> SECTOR_QUALITY_PRECISION
}

/// Returns the quality-adjusted power for a sector.
pub fn qa_power_for_sector(size: SectorSize, sector: &SectorOnChainInfo) -> StoragePower {
    let duration = sector.expiration - sector.power_base_epoch;
    qa_power_for_weight(size, duration, &sector.deal_weight, &sector.verified_deal_weight)
}

/// Determine maximum number of deal miner's sector can hold
pub fn sector_deals_max(policy: &Policy, size: SectorSize) -> u64 {
    cmp::max(256, size as u64 / policy.deal_limit_denominator)
}

/// Specification for a linear vesting schedule.
pub struct VestSpec {
    /// Delay before any amount starts vesting.
    pub initial_delay: ChainEpoch,
    /// Period over which the total should vest, after the initial delay.
    pub vest_period: ChainEpoch,
    /// Duration between successive incremental vests (independent of vesting period).
    pub step_duration: ChainEpoch,
    /// Maximum precision of vesting table (limits cardinality of table).
    pub quantization: ChainEpoch,
}

pub const REWARD_VESTING_SPEC: VestSpec = VestSpec {
    initial_delay: 0,                  // PARAM_FINISH
    vest_period: 180 * EPOCHS_IN_DAY,  // PARAM_FINISH
    step_duration: EPOCHS_IN_DAY,      // PARAM_FINISH
    quantization: 12 * EPOCHS_IN_HOUR, // PARAM_FINISH
};

// Default share of block reward allocated as reward to the consensus fault reporter.
// Applied as epochReward / (expectedLeadersPerEpoch * consensusFaultReporterDefaultShare)
pub const CONSENSUS_FAULT_REPORTER_DEFAULT_SHARE: u64 = 4;

pub fn reward_for_consensus_slash_report(epoch_reward: &TokenAmount) -> TokenAmount {
    epoch_reward.div_floor(EXPECTED_LEADERS_PER_EPOCH * CONSENSUS_FAULT_REPORTER_DEFAULT_SHARE)
}

// The reward given for successfully disputing a window post.
pub fn reward_for_disputed_window_post(
    _proof_type: RegisteredPoStProof,
    _disputed_power: PowerPair,
) -> TokenAmount {
    // This is currently just the base. In the future, the fee may scale based on the disputed power.
    BASE_REWARD_FOR_DISPUTED_WINDOW_POST.clone()
}
