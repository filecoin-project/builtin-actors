use cid::Cid;
use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_encoding::tuple::*;

#[derive(Serialize_tuple, Deserialize_tuple, Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub delegate_to: Option<EthAddress>,
    pub auth_nonce: u64,
    pub evm_storage_root: Cid,
}
