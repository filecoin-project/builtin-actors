use cid::Cid;
use fil_actors_evm_shared::address::EthAddress;
use fil_actors_runtime::{ActorDowncast, ActorError, Map, make_empty_map, make_map_with_root_and_bitwidth};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::serde as fserde;
use fvm_ipld_hamt::BytesKey;
use fvm_shared::error::ExitCode;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct State {
    pub mappings: Cid,
    pub nonces: Cid,
    pub storage_roots: Cid,
}

impl State {
    pub fn new<BS: Blockstore>(store: &BS) -> Result<Self, ActorError> {
        let mut map: Map<'_, BS, EthAddress> = make_empty_map(store, 5);
        let mut nonces: Map<'_, BS, u64> = make_empty_map(store, 5);
        let mut roots: Map<'_, BS, Cid> = make_empty_map(store, 5);
        Ok(Self {
            mappings: map.flush().map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "flush new mapping"))?,
            nonces: nonces.flush().map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "flush new nonces"))?,
            storage_roots: roots.flush().map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "flush new storage roots"))?,
        })
    }

    pub fn load_mapping<'bs, BS: Blockstore>(&'bs self, store: &'bs BS) -> Result<Map<'bs, BS, EthAddress>, ActorError> {
        make_map_with_root_and_bitwidth(&self.mappings, store, 5).map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "load mapping"))
    }

    pub fn load_nonces<'bs, BS: Blockstore>(&'bs self, store: &'bs BS) -> Result<Map<'bs, BS, u64>, ActorError> {
        make_map_with_root_and_bitwidth(&self.nonces, store, 5).map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "load nonces"))
    }

    pub fn load_storage_roots<'bs, BS: Blockstore>(&'bs self, store: &'bs BS) -> Result<Map<'bs, BS, Cid>, ActorError> {
        make_map_with_root_and_bitwidth(&self.storage_roots, store, 5)
            .map_err(|e| e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "load storage roots"))
    }
}
