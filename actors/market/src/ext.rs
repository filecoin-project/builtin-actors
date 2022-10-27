use fvm_ipld_encoding::serde_bytes;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_shared::bigint::bigint_ser;
use fvm_shared::econ::TokenAmount;

use fvm_shared::sector::StoragePower;
use fvm_shared::smooth::FilterEstimate;

pub mod account {
    use super::*;

    pub const AUTHENTICATE_MESSAGE_METHOD: u64 = 42;

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct AuthenticateMessageParams {
        #[serde(with = "serde_bytes")]
        pub signature: Vec<u8>,
        #[serde(with = "serde_bytes")]
        pub message: Vec<u8>,
    }
}

pub mod miner {
    use super::*;

    pub const CONTROL_ADDRESSES_METHOD: u64 = 2;

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct GetControlAddressesReturnParams {
        pub owner: Address,
        pub worker: Address,
        pub control_addresses: Vec<Address>,
    }
}

pub mod verifreg {
    use super::*;

    // based on fil_actor_verifreg
    pub const USE_BYTES_METHOD: u64 = 5;
    pub const RESTORE_BYTES_METHOD: u64 = 6;

    pub type UseBytesParams = BytesParams;
    pub type RestoreBytesParams = BytesParams;

    #[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
    pub struct BytesParams {
        /// Address of verified client.
        pub address: Address,
        /// Number of bytes to use.
        #[serde(with = "bigint_ser")]
        pub deal_size: StoragePower,
    }
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
