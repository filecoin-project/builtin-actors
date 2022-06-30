use fvm_ipld_encoding::{tuple::*, Cbor};

// TODO: Implement atomic swap.

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct AtomicExec {
    pub dummy: u64,
}

impl Cbor for AtomicExec {}
