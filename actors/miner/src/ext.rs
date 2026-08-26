use cid::Cid;
use fvm_ipld_encoding::RawBytes;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::ActorID;
use fvm_shared::bigint::bigint_ser;
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

// FIP-1249: verifreg ext module removed. The miner actor no longer interacts
// with the verified registry for allocation claims or claim validation.
