use fvm_ipld_encoding::strict_bytes;
use fvm_ipld_encoding::tuple::*;

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
