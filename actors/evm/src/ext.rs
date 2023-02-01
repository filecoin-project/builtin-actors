pub mod eam {
    use crate::interpreter::address::EthAddress;
    use fvm_ipld_encoding::{strict_bytes, tuple::*};
    use fvm_shared::address::Address;

    pub const CREATE_METHOD_NUM: u64 = 2;
    pub const CREATE2_METHOD_NUM: u64 = 3;

    #[derive(Serialize_tuple, Deserialize_tuple, Clone)]
    pub struct CreateParams {
        #[serde(with = "strict_bytes")]
        pub code: Vec<u8>,
        pub nonce: u64,
    }

    #[derive(Serialize_tuple, Deserialize_tuple, Clone)]
    pub struct Create2Params {
        #[serde(with = "strict_bytes")]
        pub code: Vec<u8>,
        #[serde(with = "strict_bytes")]
        pub salt: [u8; 32],
    }

    #[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CreateReturn {
        pub actor_id: u64,
        pub robust_address: Option<Address>,
        pub eth_address: EthAddress,
    }
}
