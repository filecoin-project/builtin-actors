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

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct ConstructorParams {
        pub bytecode: RawBytes,
        pub input_data: RawBytes,
    }

    pub const RESURRECT_METHOD: u64 = 7;
}
