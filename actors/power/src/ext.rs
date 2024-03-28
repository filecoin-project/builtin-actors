use cid::Cid;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{strict_bytes, BytesDe};

use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser;
use fvm_shared::sector::{RegisteredPoStProof, SectorNumber, StoragePower};
use fvm_shared::METHOD_CONSTRUCTOR;
use num_derive::FromPrimitive;

use fil_actors_runtime::reward::FilterEstimate;

pub mod init {
    use super::*;
    use fvm_ipld_encoding::RawBytes;

    pub const EXEC_METHOD: u64 = 2;

    /// Init actor Exec Params
    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct ExecParams {
        pub code_cid: Cid,
        pub constructor_params: RawBytes,
    }

    /// Init actor Exec Return value
    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct ExecReturn {
        /// ID based address for created actor
        pub id_address: Address,
        /// Reorg safe address for actor
        pub robust_address: Address,
    }
}

pub mod miner {
    use std::cmp;

    use super::*;
    use fil_actors_runtime::{
        reward::{math::PRECISION, smooth},
        DealWeight, EPOCHS_IN_DAY,
    };
    use fvm_shared::{
        bigint::{BigInt, Integer},
        clock::ChainEpoch,
        econ::TokenAmount,
        sector::SectorSize,
    };
    use lazy_static::lazy_static;
    use num_traits::Zero;

    lazy_static! {
        /// Quality multiplier for committed capacity (no deals) in a sector
        pub static ref QUALITY_BASE_MULTIPLIER: BigInt = BigInt::from(10);

        /// Quality multiplier for unverified deals in a sector
        pub static ref DEAL_WEIGHT_MULTIPLIER: BigInt = BigInt::from(10);

        /// Quality multiplier for verified deals in a sector
        pub static ref VERIFIED_DEAL_WEIGHT_MULTIPLIER: BigInt = BigInt::from(100);

        /// Cap on initial pledge requirement for sectors during the Space Race network.
        /// The target is 1 FIL (10**18 attoFIL) per 32GiB.
        /// This does not divide evenly, so the result is fractionally smaller.
        static ref INITIAL_PLEDGE_MAX_PER_BYTE: TokenAmount =
            TokenAmount::from_whole(1).div_floor(32i64 << 30);
    }

    /// Precision used for making QA power calculations
    pub const SECTOR_QUALITY_PRECISION: i64 = 20;

    /// Projection period of expected sector block rewards for storage pledge required to commit a sector.
    /// This pledge is lost if a sector is terminated before its full committed lifetime.
    pub const INITIAL_PLEDGE_FACTOR: u64 = 20;

    pub const INITIAL_PLEDGE_PROJECTION_PERIOD: i64 =
        (INITIAL_PLEDGE_FACTOR as ChainEpoch) * EPOCHS_IN_DAY;

    const LOCK_TARGET_FACTOR_NUM: u32 = 3;
    const LOCK_TARGET_FACTOR_DENOM: u32 = 10;

    pub const CONFIRM_SECTOR_PROOFS_VALID_METHOD: u64 = 17;
    pub const ON_DEFERRED_CRON_EVENT_METHOD: u64 = 12;
    pub const LOCK_CREATE_MINER_DESPOIT_METHOD: u64 =
        frc42_dispatch::method_hash!("LockCreateMinerDeposit");

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct ConfirmSectorProofsParams {
        pub sectors: Vec<SectorNumber>,
        pub reward_smoothed: FilterEstimate,
        #[serde(with = "bigint_ser")]
        pub reward_baseline_power: StoragePower,
        pub quality_adj_power_smoothed: FilterEstimate,
    }

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct MinerConstructorParams {
        pub owner: Address,
        pub worker: Address,
        pub control_addresses: Vec<Address>,
        pub window_post_proof_type: RegisteredPoStProof,
        #[serde(with = "strict_bytes")]
        pub peer_id: Vec<u8>,
        pub multi_addresses: Vec<BytesDe>,
    }

    /// Copy from miner
    ///
    /// Network inputs to calculation of sector pledge and associated parameters.
    pub struct MinerNetworkPledgeInputs {
        pub network_qap: FilterEstimate,
        pub network_baseline: StoragePower,
        pub circulating_supply: TokenAmount,
        pub epoch_reward: FilterEstimate,
    }

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct DeferredCronEventParams {
        #[serde(with = "strict_bytes")]
        pub event_payload: Vec<u8>,
        pub reward_smoothed: FilterEstimate,
        pub quality_adj_power_smoothed: FilterEstimate,
    }

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct LockCreateMinerDepositParams {
        pub amount: TokenAmount,
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

    /// DealWeight and VerifiedDealWeight are spacetime occupied by regular deals and verified deals in a sector.
    /// Sum of DealWeight and VerifiedDealWeight should be less than or equal to total SpaceTime of a sector.
    /// Sectors full of VerifiedDeals will have a BigInt of VerifiedDealWeightMultiplier/QualityBaseMultiplier.
    /// Sectors full of Deals will have a BigInt of DealWeightMultiplier/QualityBaseMultiplier.
    /// Sectors with neither will have a BigInt of QualityBaseMultiplier/QualityBaseMultiplier.
    /// BigInt of a sector is a weighted average of multipliers based on their proportions.
    pub fn quality_for_weight(
        size: SectorSize,
        duration: ChainEpoch,
        deal_weight: &DealWeight,
        verified_weight: &DealWeight,
    ) -> BigInt {
        let sector_space_time = BigInt::from(size as u64) * BigInt::from(duration);
        let total_deal_space_time = deal_weight + verified_weight;

        let weighted_base_space_time =
            (&sector_space_time - total_deal_space_time) * &*QUALITY_BASE_MULTIPLIER;
        let weighted_deal_space_time = deal_weight * &*DEAL_WEIGHT_MULTIPLIER;
        let weighted_verified_space_time = verified_weight * &*VERIFIED_DEAL_WEIGHT_MULTIPLIER;
        let weighted_sum_space_time =
            weighted_base_space_time + weighted_deal_space_time + weighted_verified_space_time;
        let scaled_up_weighted_sum_space_time: BigInt =
            weighted_sum_space_time << SECTOR_QUALITY_PRECISION;

        scaled_up_weighted_sum_space_time
            .div_floor(&sector_space_time)
            .div_floor(&QUALITY_BASE_MULTIPLIER)
    }

    /// Computes the pledge requirement for committing new quality-adjusted power to the network, given
    /// the current network total and baseline power, per-epoch  reward, and circulating token supply.
    /// The pledge comprises two parts:
    /// - storage pledge, aka IP base: a multiple of the reward expected to be earned by newly-committed power
    /// - consensus pledge, aka additional IP: a pro-rata fraction of the circulating money supply
    ///
    /// IP = IPBase(t) + AdditionalIP(t)
    /// IPBase(t) = BR(t, InitialPledgeProjectionPeriod)
    /// AdditionalIP(t) = LockTarget(t)*PledgeShare(t)
    /// LockTarget = (LockTargetFactorNum / LockTargetFactorDenom) * FILCirculatingSupply(t)
    /// PledgeShare(t) = sectorQAPower / max(BaselinePower(t), NetworkQAPower(t))
    pub fn initial_pledge_for_power(
        qa_power: &StoragePower,
        baseline_power: &StoragePower,
        reward_estimate: &FilterEstimate,
        network_qa_power_estimate: &FilterEstimate,
        circulating_supply: &TokenAmount,
    ) -> TokenAmount {
        let ip_base = expected_reward_for_power_clamped_at_atto_fil(
            reward_estimate,
            network_qa_power_estimate,
            qa_power,
            INITIAL_PLEDGE_PROJECTION_PERIOD,
        );

        let lock_target_num = circulating_supply.atto() * LOCK_TARGET_FACTOR_NUM;
        let lock_target_denom = LOCK_TARGET_FACTOR_DENOM;
        let pledge_share_num = qa_power;
        let network_qa_power = network_qa_power_estimate.estimate();
        let pledge_share_denom = cmp::max(cmp::max(&network_qa_power, baseline_power), qa_power);
        let additional_ip_num = lock_target_num * pledge_share_num;
        let additional_ip_denom = pledge_share_denom * lock_target_denom;
        let additional_ip = additional_ip_num.div_floor(&additional_ip_denom);

        let nominal_pledge = ip_base + TokenAmount::from_atto(additional_ip);
        let pledge_cap = TokenAmount::from_atto(INITIAL_PLEDGE_MAX_PER_BYTE.atto() * qa_power);

        cmp::min(nominal_pledge, pledge_cap)
    }

    /// The projected block reward a sector would earn over some period.
    /// Also known as "BR(t)".
    /// BR(t) = ProjectedRewardFraction(t) * SectorQualityAdjustedPower
    /// ProjectedRewardFraction(t) is the sum of estimated reward over estimated total power
    /// over all epochs in the projection period [t t+projectionDuration]
    pub fn expected_reward_for_power(
        reward_estimate: &FilterEstimate,
        network_qa_power_estimate: &FilterEstimate,
        qa_sector_power: &StoragePower,
        projection_duration: ChainEpoch,
    ) -> TokenAmount {
        let network_qa_power_smoothed = network_qa_power_estimate.estimate();

        if network_qa_power_smoothed.is_zero() {
            return TokenAmount::from_atto(reward_estimate.estimate());
        }

        let expected_reward_for_proving_period = smooth::extrapolated_cum_sum_of_ratio(
            projection_duration,
            0,
            reward_estimate,
            network_qa_power_estimate,
        );
        let br128 = qa_sector_power * expected_reward_for_proving_period; // Q.0 * Q.128 => Q.128
        TokenAmount::from_atto(std::cmp::max(br128 >> PRECISION, Default::default()))
    }

    // BR but zero values are clamped at 1 attofil
    // Some uses of BR (PCD, IP) require a strictly positive value for BR derived values so
    // accounting variables can be used as succinct indicators of miner activity.
    pub fn expected_reward_for_power_clamped_at_atto_fil(
        reward_estimate: &FilterEstimate,
        network_qa_power_estimate: &FilterEstimate,
        qa_sector_power: &StoragePower,
        projection_duration: ChainEpoch,
    ) -> TokenAmount {
        let br = expected_reward_for_power(
            reward_estimate,
            network_qa_power_estimate,
            qa_sector_power,
            projection_duration,
        );
        if br.le(&TokenAmount::zero()) {
            TokenAmount::from_atto(1)
        } else {
            br
        }
    }
}

pub mod reward {
    use super::*;

    pub const THIS_EPOCH_REWARD_METHOD: u64 = 3;
    pub const UPDATE_NETWORK_KPI: u64 = 4;

    #[derive(FromPrimitive)]
    #[repr(u64)]
    pub enum Method {
        Constructor = METHOD_CONSTRUCTOR,
        AwardBlockReward = 2,
        ThisEpochReward = 3,
        UpdateNetworkKPI = 4,
    }
}
