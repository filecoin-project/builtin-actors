use {
    cid::Cid,
    fil_actors_runtime::make_empty_map,
    fvm_ipld_blockstore::Blockstore,
    fvm_ipld_encoding::{Cbor, CborStore, RawBytes},
    fvm_ipld_encoding::tuple::*,
    fvm_ipld_blockstore::Block,
    fvm_ipld_hamt::Hamt,
    fvm_shared::HAMT_BIT_WIDTH,
    multihash::Code,
    serde_tuple::{Deserialize_tuple, Serialize_tuple},
    crate::interpreter::U256,
};

pub const RAW: u64 = 0x55;

/// Data stored by an EVM contract.
/// This runs on the fvm-evm-runtime actor code cid.
#[derive(Debug, Serialize_tuple, Deserialize_tuple)]
pub struct State {
    /// The EVM contract bytecode resulting from calling the
    /// initialization code by the constructor.
    pub bytecode: Cid,

    /// The EVM contract state dictionary.
    /// All eth contract state is a map of U256 -> U256 values.
    ///
    /// HAMT<U256, U256>
    pub contract_state: Cid,
}

impl Cbor for State {}

impl State {
    pub fn new<BS: Blockstore>(
        store: &BS,
        bytecode: RawBytes,
    ) -> anyhow::Result<Self> {
        let bytecode_cid = store.put(
            Code::Blake2b256,
            &Block::new(RAW, bytecode.to_vec()),
        )?;
        let contract_state_hamt: Hamt<_, U256> = make_empty_map(&store, HAMT_BIT_WIDTH);
        let contract_state_cid = store.put_cbor(&contract_state_hamt, Code::Blake2b256)?;
        Ok(Self {
            bytecode: bytecode_cid,
            contract_state: contract_state_cid,
        })
    }
}
