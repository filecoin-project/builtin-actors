use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_encoding::serde; 
use fvm_ipld_encoding::strict_bytes;
use fvm_ipld_encoding::tuple::{Deserialize_tuple, Serialize_tuple};
// Ensure derive helper path exists
use fvm_ipld_encoding::tuple::serde_tuple;

#[derive(Debug, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct DelegationParam {
    pub chain_id: u64,
    pub address: EthAddress,
    pub nonce: u64,
    pub y_parity: u8,
    #[serde(with = "strict_bytes")]
    pub r: [u8; 32],
    #[serde(with = "strict_bytes")]
    pub s: [u8; 32],
}

#[derive(Debug, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct ApplyDelegationsParams {
    pub list: Vec<DelegationParam>,
}

#[derive(Debug, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct LookupDelegateParams {
    pub authority: EthAddress,
}

#[derive(Debug, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct LookupDelegateReturn {
    pub delegate: Option<EthAddress>,
}

#[derive(Debug, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct GetStorageRootParams {
    pub authority: EthAddress,
}

#[derive(Debug, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct GetStorageRootReturn {
    pub root: Option<cid::Cid>,
}

#[derive(Debug, Clone, Serialize_tuple, Deserialize_tuple)]
pub struct PutStorageRootParams {
    pub authority: EthAddress,
    pub root: cid::Cid,
}
