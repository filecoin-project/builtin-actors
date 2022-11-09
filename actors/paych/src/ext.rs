use fvm_ipld_encoding::serde_bytes;
use fvm_ipld_encoding::tuple::*;

pub mod account {
    use super::*;

    pub const AUTHENTICATE_MESSAGE_METHOD: u64 = 3;

    #[derive(Serialize_tuple, Deserialize_tuple)]
    pub struct AuthenticateMessageParams {
        #[serde(with = "serde_bytes")]
        pub signature: Vec<u8>,
        #[serde(with = "serde_bytes")]
        pub message: Vec<u8>,
    }
}
