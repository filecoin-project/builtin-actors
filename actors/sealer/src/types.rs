use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use std::marker::PhantomData;

#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ConstructorParams {
    // No real parameters yet; this dummy field stops the derive macro from
    // generating an unused-lifetime error.
    #[serde(skip)]
    _phantom: PhantomData<()>,
}

#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct SealerIDReturn {
    pub id_addr: Address,
} 