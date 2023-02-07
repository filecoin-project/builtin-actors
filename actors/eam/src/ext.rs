use cid::Cid;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::address::Address;

pub mod init {
    use super::*;

    pub const EXEC4_METHOD: u64 = 3;

    /// Init actor Exec4 Params
    #[derive(Serialize_tuple, Deserialize_tuple, Debug)]
    pub struct Exec4Params {
        pub code_cid: Cid,
        pub constructor_params: RawBytes,
        pub subaddress: RawBytes,
    }

    /// Init actor Exec4 Return value
    #[derive(Serialize_tuple, Deserialize_tuple, Debug)]
    pub struct Exec4Return {
        /// ID based address for created actor
        pub id_address: Address,
        /// Reorg safe address for actor
        pub robust_address: Address,
    }
}

pub mod evm {
    use super::*;

    use fil_actors_evm_shared::address::EthAddress;

    #[derive(Serialize_tuple, Deserialize_tuple, Clone)]
    pub struct ConstructorParams {
        /// The actor's "creator" (specified by the EAM).
        pub creator: EthAddress,
        /// The initcode that will construct the new EVM actor.
        pub initcode: RawBytes,
    }

    pub const RESURRECT_METHOD: u64 = 2;
}

pub mod account {
    pub const PUBKEY_ADDRESS_METHOD: u64 = 2;
}
