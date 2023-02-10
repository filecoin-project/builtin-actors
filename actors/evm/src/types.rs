use fil_actors_evm_shared::address::EthAddress;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::RawBytes;

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct ConstructorParams {
    /// The actor's "creator" (specified by the EAM).
    pub creator: EthAddress,
    /// The initcode that will construct the new EVM actor.
    pub initcode: RawBytes,
}
