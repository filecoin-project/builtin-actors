use cid::Cid;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_evm_shared::uints::U256;
use fvm_ipld_encoding::strict_bytes;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::econ::TokenAmount;

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ConstructorParams {
    /// The actor's "creator" (specified by the EAM).
    pub creator: EthAddress,
    /// The initcode that will construct the new EVM actor.
    pub initcode: RawBytes,
}

pub type ResurrectParams = ConstructorParams;

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct DelegateCallParams {
    pub code: Cid,
    /// The contract invocation parameters
    #[serde(with = "strict_bytes")]
    pub input: Vec<u8>,
    /// The original caller's Eth address.
    pub caller: EthAddress,
    /// The value passed in the original call.
    pub value: TokenAmount,
}

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct GetStorageAtParams {
    pub storage_key: U256,
}
