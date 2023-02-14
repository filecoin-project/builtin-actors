use fvm_ipld_encoding::strict_bytes;
use fvm_ipld_encoding::tuple::*;

#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct AuthenticateMessageParams {
    #[serde(with = "strict_bytes")]
    pub signature: Vec<u8>,
    #[serde(with = "strict_bytes")]
    pub message: Vec<u8>,
}
