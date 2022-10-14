#![allow(dead_code)]

use fil_actors_runtime::{actor_error, runtime::EMPTY_ARR_CID, AsActorError, EAM_ACTOR_ID};
use fvm_ipld_blockstore::Block;
use fvm_ipld_encoding::{CborStore, RawBytes};
use fvm_shared::{
    address::{Address, Payload},
    econ::TokenAmount,
    error::ExitCode,
    MethodNum, IPLD_RAW,
};
use multihash::Code;

use crate::state::State;

use super::{address::EthAddress, Bytecode};

use {
    crate::interpreter::{StatusCode, U256},
    cid::Cid,
    fil_actors_runtime::{runtime::Runtime, ActorError},
    fvm_ipld_blockstore::Blockstore,
    fvm_ipld_hamt::Hamt,
};

#[derive(Clone, Copy, Debug)]
pub enum StorageStatus {
    /// The value of a storage item has been left unchanged: 0 -> 0 and X -> X.
    Unchanged,
    /// The value of a storage item has been modified: X -> Y.
    Modified,
    /// A storage item has been modified after being modified before: X -> Y -> Z.
    ModifiedAgain,
    /// A new storage item has been added: 0 -> X.
    Added,
    /// A storage item has been deleted: X -> 0.
    Deleted,
}

/// Platform Abstraction Layer
/// that bridges the FVM world to EVM world
pub struct System<'r, BS: Blockstore, RT: Runtime<BS>> {
    pub rt: &'r mut RT,

    bytecode: Option<Cid>,
    slots: Hamt<BS, U256, U256>,
    nonce: u64,
    saved_state_root: Option<Cid>,
}

impl<'r, BS: Blockstore, RT: Runtime<BS>> System<'r, BS, RT> {
    /// Create the actor.
    pub fn create(rt: &'r mut RT) -> Result<Self, ActorError>
    where
        BS: Clone,
    {
        let state_root = rt.get_state_root()?;
        if state_root != EMPTY_ARR_CID {
            return Err(actor_error!(illegal_state, "can't create over an existing actor"));
        }
        let store = rt.store().clone();
        Ok(Self {
            rt,
            slots: Hamt::<_, U256, U256>::new(store),
            nonce: 1,
            saved_state_root: None,
            bytecode: None,
        })
    }

    /// Load the actor from state.
    pub fn load(rt: &'r mut RT) -> Result<Self, ActorError>
    where
        BS: Clone,
    {
        let store = rt.store().clone();
        let state_root = rt.get_state_root()?;
        let state: State = store
            .get_cbor(&state_root)
            .context_code(ExitCode::USR_SERIALIZATION, "failed to decode state")?
            .context_code(ExitCode::USR_ILLEGAL_STATE, "state not in blockstore")?;

        Ok(Self {
            rt,
            slots: Hamt::<_, U256, U256>::load(&state.contract_state, store)
                .context_code(ExitCode::USR_ILLEGAL_STATE, "state not in blockstore")?,
            nonce: state.nonce,
            saved_state_root: Some(state_root),
            bytecode: Some(state.bytecode),
        })
    }

    pub fn increment_nonce(&mut self) -> u64 {
        self.saved_state_root = None;
        let nonce = self.nonce;
        self.nonce = self.nonce.checked_add(1).unwrap();
        nonce
    }

    /// Send a message, saving and reloading state as necessary.
    pub fn send(
        &mut self,
        to: &Address,
        method: MethodNum,
        params: RawBytes,
        value: TokenAmount,
    ) -> Result<RawBytes, ActorError> {
        self.flush()?;
        let result = self.rt.send(to, method, params, value)?;
        self.reload()?;
        Ok(result)
    }

    /// Flush the actor state (bytecode, nonce, and slots).
    pub fn flush(&mut self) -> Result<(), ActorError> {
        if self.saved_state_root.is_some() {
            return Ok(());
        }
        let bytecode_cid = match self.bytecode {
            Some(cid) => cid,
            None => self.set_bytecode(&[])?,
        };
        let new_root = self
            .rt
            .store()
            .put_cbor(
                &State {
                    bytecode: bytecode_cid,
                    contract_state: self.slots.flush().context_code(
                        ExitCode::USR_ILLEGAL_STATE,
                        "failed to flush contract state",
                    )?,
                    nonce: self.nonce,
                },
                Code::Blake2b256,
            )
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to write contract state")?;

        self.rt.set_state_root(&new_root)?;
        self.saved_state_root = Some(new_root);
        Ok(())
    }

    /// Reload the actor state if changed.
    pub fn reload(&mut self) -> Result<(), ActorError> {
        let root = self.rt.get_state_root()?;
        if self.saved_state_root == Some(root) {
            return Ok(());
        }
        let state: State = self
            .rt
            .store()
            .get_cbor(&root)
            .context_code(ExitCode::USR_SERIALIZATION, "failed to decode state")?
            .context_code(ExitCode::USR_ILLEGAL_STATE, "state not in blockstore")?;

        self.slots
            .set_root(&state.contract_state)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "state not in blockstore")?;
        self.nonce = state.nonce;
        self.saved_state_root = Some(root);
        self.bytecode = Some(state.bytecode);
        Ok(())
    }

    /// Load the bytecode.
    pub fn load_bytecode(&self) -> Result<Option<Bytecode>, ActorError> {
        match &self.bytecode {
            Some(cid) => {
                let bytecode = self
                    .rt
                    .store()
                    .get(cid)
                    .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to read state")?
                    .expect("bytecode not in state tree");
                if bytecode.is_empty() {
                    return Ok(None);
                }
                Ok(Some(Bytecode::new(bytecode)))
            }
            None => Ok(None),
        }
    }

    /// Set the bytecode.
    pub fn set_bytecode(&mut self, bytecode: &[u8]) -> Result<Cid, ActorError> {
        self.saved_state_root = None;
        // Reject code starting with 0xEF, EIP-3541
        if bytecode.first() == Some(&0xEF) {
            return Err(ActorError::illegal_argument(
                "EIP-3541: Contract code starting with the 0xEF byte is disallowed.".into(),
            ));
        }
        let k = self
            .rt
            .store()
            .put(Code::Blake2b256, &Block::new(IPLD_RAW, bytecode))
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to write bytecode")?;
        self.bytecode = Some(k);
        Ok(k)
    }

    /// Get value of a storage key.
    pub fn get_storage(&mut self, key: U256) -> Result<Option<U256>, StatusCode> {
        let mut key_bytes = [0u8; 32];
        key.to_big_endian(&mut key_bytes);

        Ok(self.slots.get(&key).map_err(|e| StatusCode::InternalError(e.to_string()))?.cloned())
    }

    /// Set value of a storage key.
    pub fn set_storage(
        &mut self,
        key: U256,
        value: Option<U256>,
    ) -> Result<StorageStatus, StatusCode> {
        self.saved_state_root = None; // dirty.

        let mut key_bytes = [0u8; 32];
        key.to_big_endian(&mut key_bytes);

        let prev_value =
            self.slots.get(&key).map_err(|e| StatusCode::InternalError(e.to_string()))?.cloned();

        let mut storage_status =
            if prev_value == value { StorageStatus::Unchanged } else { StorageStatus::Modified };

        if value.is_none() {
            self.slots.delete(&key).map_err(|e| StatusCode::InternalError(e.to_string()))?;
            storage_status = StorageStatus::Deleted;
        } else {
            self.slots
                .set(key, value.unwrap())
                .map_err(|e| StatusCode::InternalError(e.to_string()))?;
        }

        Ok(storage_status)
    }

    /// Resolve the address to the ethereum equivalent, if possible.
    pub fn resolve_ethereum_address(&self, addr: &Address) -> Result<EthAddress, StatusCode> {
        // Short-circuit if we already have an EVM actor.
        match addr.payload() {
            Payload::Delegated(delegated) if delegated.namespace() == EAM_ACTOR_ID => {
                let subaddr: [u8; 20] = delegated.subaddress().try_into().map_err(|_| {
                    StatusCode::BadAddress("invalid ethereum address length".into())
                })?;
                return Ok(EthAddress(subaddr));
            }
            _ => {}
        }

        // Otherwise, resolve to an ID address.
        let actor_id = self.rt.resolve_address(addr).ok_or_else(|| {
            StatusCode::BadAddress(format!(
                "non-ethereum address {addr} cannot be resolved to an ID address"
            ))
        })?;

        // Then attempt to resolve back into an EVM address.
        //
        // TODO: this method doesn't differentiate between "actor doesn't have a predictable
        // address" and "actor doesn't exist". We should probably fix that and return an error if
        // the actor doesn't exist.
        match self.rt.lookup_address(actor_id).map(|a| a.into_payload()) {
            Some(Payload::Delegated(delegated)) if delegated.namespace() == EAM_ACTOR_ID => {
                let subaddr: [u8; 20] = delegated.subaddress().try_into().map_err(|_| {
                    StatusCode::BadAddress("invalid ethereum address length".into())
                })?;
                Ok(EthAddress(subaddr))
            }
            // But use an EVM address as the fallback.
            _ => Ok(EthAddress::from_id(actor_id)),
        }
    }
}
