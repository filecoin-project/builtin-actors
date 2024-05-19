use cid::Cid;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::{strict_bytes, BytesDe};

use fvm_shared::address::Address;
use fvm_shared::sector::RegisteredPoStProof;
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
    use super::*;

    pub const ON_DEFERRED_CRON_EVENT_METHOD: u64 = 12;

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

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct DeferredCronEventParams {
        #[serde(with = "strict_bytes")]
        pub event_payload: Vec<u8>,
        pub reward_smoothed: FilterEstimate,
        pub quality_adj_power_smoothed: FilterEstimate,
    }
}

pub mod reward {
    use super::*;

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
