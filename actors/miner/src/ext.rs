use cid::Cid;
use fvm_ipld_encoding::RawBytes;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::ActorID;
use fvm_shared::bigint::{BigInt, bigint_ser};
use fvm_shared::clock::ChainEpoch;
use fvm_shared::deal::DealID;
use fvm_shared::econ::TokenAmount;
use fvm_shared::piece::PaddedPieceSize;
use fvm_shared::sector::SectorNumber;
use fvm_shared::sector::{RegisteredSealProof, StoragePower};

use fil_actors_runtime::BatchReturn;
use fil_actors_runtime::reward::FilterEstimate;

pub mod account {
    pub const PUBKEY_ADDRESS_METHOD: u64 = 2;
}

pub mod market {
    use super::*;
    use fvm_ipld_bitfield::BitField;

    pub const VERIFY_DEALS_FOR_ACTIVATION_METHOD: u64 = 5;
    pub const BATCH_ACTIVATE_DEALS_METHOD: u64 = 6;
    pub const ON_MINER_SECTORS_TERMINATE_METHOD: u64 = 7;

    pub const NO_ALLOCATION_ID: u64 = 0;

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct SectorDeals {
        pub sector_number: SectorNumber,
        pub sector_type: RegisteredSealProof,
        pub sector_expiry: ChainEpoch,
        pub deal_ids: Vec<DealID>,
    }

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct BatchActivateDealsParams {
        pub sectors: Vec<SectorDeals>,
        pub compute_cid: bool,
    }

    #[derive(Serialize_tuple, Deserialize_tuple, Clone)]
    pub struct ActivatedDeal {
        pub client: ActorID,
        pub allocation_id: u64,
        pub data: Cid,
        pub size: PaddedPieceSize,
    }

    #[derive(Serialize_tuple, Deserialize_tuple, Clone)]
    pub struct SectorDealActivation {
        pub activated: Vec<ActivatedDeal>,
        pub unsealed_cid: Option<Cid>,
    }

    #[derive(Serialize_tuple, Deserialize_tuple, Clone)]
    pub struct BatchActivateDealsResult {
        pub activation_results: BatchReturn,
        pub activations: Vec<SectorDealActivation>,
    }

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct OnMinerSectorsTerminateParams {
        pub epoch: ChainEpoch,
        pub sectors: BitField,
    }

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct SectorDataSpec {
        pub deal_ids: Vec<DealID>,
        pub sector_type: RegisteredSealProof,
    }

    #[derive(Serialize_tuple)]
    pub struct VerifyDealsForActivationParamsRef<'a> {
        pub sectors: &'a [SectorDeals],
    }

    #[derive(Serialize_tuple, Deserialize_tuple, Default, Clone)]
    pub struct VerifyDealsForActivationReturn {
        pub unsealed_cids: Vec<Option<Cid>>,
    }
}

pub mod power {
    use super::*;

    pub const UPDATE_CLAIMED_POWER_METHOD: u64 = 3;
    pub const ENROLL_CRON_EVENT_METHOD: u64 = 4;
    pub const UPDATE_PLEDGE_TOTAL_METHOD: u64 = 6;
    pub const SUBMIT_POREP_FOR_BULK_VERIFY_METHOD: u64 = 8;
    pub const CURRENT_TOTAL_POWER_METHOD: u64 = 9;

    #[derive(Serialize_tuple, Deserialize_tuple, Default)]
    pub struct CurrentTotalPowerReturn {
        #[serde(with = "bigint_ser")]
        pub raw_byte_power: StoragePower,
        #[serde(with = "bigint_ser")]
        pub quality_adj_power: StoragePower,
        pub pledge_collateral: TokenAmount,
        pub quality_adj_power_smoothed: FilterEstimate,
        pub ramp_start_epoch: i64,
        pub ramp_duration_epochs: u64,
    }

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct EnrollCronEventParams {
        pub event_epoch: ChainEpoch,
        pub payload: RawBytes,
    }

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct UpdateClaimedPowerParams {
        #[serde(with = "bigint_ser")]
        pub raw_byte_delta: StoragePower,
        #[serde(with = "bigint_ser")]
        pub quality_adjusted_delta: StoragePower,
    }

    pub const MAX_MINER_PROVE_COMMITS_PER_EPOCH: usize = 200;
}

pub mod reward {
    pub const THIS_EPOCH_REWARD_METHOD: u64 = 3;
}

pub mod verifreg {
    use super::*;

    pub const GET_CLAIMS_METHOD: u64 = 10;
    pub const CLAIM_ALLOCATIONS_METHOD: u64 = 9;

    pub type ClaimID = u64;
    pub type AllocationID = u64;

    #[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug, PartialEq, Eq)]
    pub struct Claim {
        // The provider storing the data (from allocation).
        pub provider: ActorID,
        // The client which allocated the DataCap (from allocation).
        pub client: ActorID,
        // Identifier of the data committed (from allocation).
        pub data: Cid,
        // The (padded) size of data (from allocation).
        pub size: PaddedPieceSize,
        // The min period which the provider must commit to storing data
        pub term_min: ChainEpoch,
        // The max period for which provider can earn QA-power for the data
        pub term_max: ChainEpoch,
        // The epoch at which the (first range of the) piece was committed.
        pub term_start: ChainEpoch,
        // ID of the provider's sector in which the data is committed.
        pub sector: SectorNumber,
    }
    #[derive(Debug, Serialize_tuple, Deserialize_tuple)]
    pub struct GetClaimsParams {
        pub provider: ActorID,
        pub claim_ids: Vec<ClaimID>,
    }
    #[derive(Debug, Serialize_tuple, Deserialize_tuple)]

    pub struct GetClaimsReturn {
        pub batch_info: BatchReturn,
        pub claims: Vec<Claim>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
    pub struct SectorAllocationClaims {
        pub sector: SectorNumber,
        pub expiry: ChainEpoch,
        pub claims: Vec<AllocationClaim>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
    pub struct AllocationClaim {
        pub client: ActorID,
        pub allocation_id: AllocationID,
        pub data: Cid,
        pub size: PaddedPieceSize,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
    pub struct ClaimAllocationsParams {
        pub sectors: Vec<SectorAllocationClaims>,
        pub all_or_nothing: bool,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Default, Serialize_tuple, Deserialize_tuple)]
    #[serde(transparent)]
    pub struct SectorClaimSummary {
        #[serde(with = "bigint_ser")]
        pub claimed_space: BigInt,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
    pub struct ClaimAllocationsReturn {
        pub sector_results: BatchReturn,
        pub sector_claims: Vec<SectorClaimSummary>,
    }
}
