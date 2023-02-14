use fvm_ipld_encoding::strict_bytes;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser;
use fvm_shared::econ::TokenAmount;

use fvm_shared::sector::StoragePower;
use fvm_shared::smooth::FilterEstimate;

pub mod account {
    use super::*;

    pub const AUTHENTICATE_MESSAGE_METHOD: u64 =
        frc42_dispatch::method_hash!("AuthenticateMessage");

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct AuthenticateMessageParams {
        #[serde(with = "strict_bytes")]
        pub signature: Vec<u8>,
        #[serde(with = "strict_bytes")]
        pub message: Vec<u8>,
    }
}

pub mod miner {
    use super::*;

    pub const CONTROL_ADDRESSES_METHOD: u64 = 2;
    pub const IS_CONTROLLING_ADDRESS_EXPORTED: u64 =
        frc42_dispatch::method_hash!("IsControllingAddress");

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct GetControlAddressesReturnParams {
        pub owner: Address,
        pub worker: Address,
        pub control_addresses: Vec<Address>,
    }

    #[derive(Serialize_tuple, Deserialize_tuple)]
    #[serde(transparent)]
    pub struct IsControllingAddressReturn {
        pub is_controlling: bool,
    }

    #[derive(Serialize_tuple, Deserialize_tuple)]
    #[serde(transparent)]
    pub struct IsControllingAddressParam {
        pub address: Address,
    }
}

pub mod verifreg {
    use super::*;
    use cid::Cid;
    use fil_actors_runtime::BatchReturn;
    use fvm_shared::clock::ChainEpoch;
    use fvm_shared::piece::PaddedPieceSize;
    use fvm_shared::ActorID;

    pub type AllocationID = u64;
    pub type ClaimID = u64;

    #[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
    pub struct AllocationRequest {
        pub provider: ActorID,
        pub data: Cid,
        pub size: PaddedPieceSize,
        pub term_min: ChainEpoch,
        pub term_max: ChainEpoch,
        pub expiration: ChainEpoch,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
    pub struct ClaimExtensionRequest {
        pub provider: ActorID,
        pub claim: ClaimID,
        pub term_max: ChainEpoch,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
    pub struct AllocationRequests {
        pub allocations: Vec<AllocationRequest>,
        pub extensions: Vec<ClaimExtensionRequest>,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
    pub struct AllocationsResponse {
        // Result for each allocation request.
        pub allocation_results: BatchReturn,
        // Result for each extension request.
        pub extension_results: BatchReturn,
        // IDs of new allocations created.
        pub new_allocations: Vec<AllocationID>,
    }
}

pub mod datacap {
    pub const BALANCE_OF_METHOD: u64 = frc42_dispatch::method_hash!("Balance");
    pub const TRANSFER_FROM_METHOD: u64 = frc42_dispatch::method_hash!("TransferFrom");
}

pub mod reward {
    pub const THIS_EPOCH_REWARD_METHOD: u64 = 3;
}

pub mod power {
    use super::*;

    pub const CURRENT_TOTAL_POWER_METHOD: u64 = 9;

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct CurrentTotalPowerReturnParams {
        #[serde(with = "bigint_ser")]
        pub raw_byte_power: StoragePower,
        #[serde(with = "bigint_ser")]
        pub quality_adj_power: StoragePower,
        pub pledge_collateral: TokenAmount,
        pub quality_adj_power_smoothed: FilterEstimate,
    }
}
