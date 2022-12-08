#![allow(dead_code)]

use std::borrow::Cow;

use fil_actors_runtime::{actor_error, runtime::EMPTY_ARR_CID, AsActorError, EAM_ACTOR_ID};
use fvm_ipld_blockstore::Block;
use fvm_ipld_encoding::{CborStore, RawBytes};
use fvm_ipld_kamt::HashedKey;
use fvm_shared::{
    address::{Address, Payload},
    econ::TokenAmount,
    error::ExitCode,
    sys::SendFlags,
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
    fvm_ipld_kamt::{AsHashedKey, Config as KamtConfig, Kamt},
};

lazy_static::lazy_static! {
    // The Solidity compiler creates contiguous array item keys.
    // To prevent the tree from going very deep we use extensions,
    // which the Kamt supports and does in all cases.
    //
    // There are maximum 32 levels in the tree with the default bit width of 8.
    // The top few levels will have a higher level of overlap in their hashes.
    // Intuitively these levels should be used for routing, not storing data.
    //
    // The only exception to this is the top level variables in the contract
    // which solidity puts in the first few slots. There having to do extra
    // lookups is burdensome, and they will always be accessed even for arrays
    // because that's where the array length is stored.
    //
    // However, for Solidity, the size of the KV pairs is 2x256, which is
    // comparable to a size of a CID pointer plus extension metadata.
    // We can keep the root small either by force-pushing data down,
    // or by not allowing many KV pairs in a slot.
    //
    // The following values have been set by looking at how the charts evolved
    // with the test contract. They might not be the best for other contracts.
    static ref KAMT_CONFIG: KamtConfig = KamtConfig {
        min_data_depth: 0,
        bit_width: 5,
        max_array_width: 1
    };
}

pub struct StateHashAlgorithm;

/// Wrapper around the base U256 type so we can control the byte order in the hash, because
/// the words backing `U256` are in little endian order, and we need them in big endian for
/// the nibbles to be co-located in the tree.
impl AsHashedKey<U256, 32> for StateHashAlgorithm {
    fn as_hashed_key(key: &U256) -> Cow<HashedKey<32>> {
        let mut bs = [0u8; 32];
        key.to_big_endian(&mut bs);
        Cow::Owned(bs)
    }
}

/// The EVM stores its state as Key-Value pairs with both keys and values
/// being 256 bits long, which we store in a KAMT.
pub type StateKamt<BS> = Kamt<BS, U256, U256, StateHashAlgorithm>;

/// Maximum allowed EVM bytecode size.
/// The contract code size limit is 24kB.
const MAX_CODE_SIZE: usize = 24 << 10;

/// Platform Abstraction Layer
/// that bridges the FVM world to EVM world
pub struct System<'r, RT: Runtime> {
    pub rt: &'r mut RT,

    /// The current bytecode. This is usually only "none" when the actor is first constructed.
    bytecode: Option<Cid>,
    /// The contract's EVM storage slots.
    slots: StateKamt<RT::Blockstore>,
    /// The contracts "nonce" (incremented when creating new actors).
    nonce: u64,
    /// The last saved state root. None if the current state hasn't been saved yet.
    saved_state_root: Option<Cid>,
    /// Read Only context (staticcall)
    pub readonly: bool,
}

impl<'r, RT: Runtime> System<'r, RT> {
    /// Create the actor.
    pub fn create(rt: &'r mut RT) -> Result<Self, ActorError>
    where
        RT::Blockstore: Clone,
    {
        let read_only = rt.read_only();
        let state_root = rt.get_state_root()?;
        if state_root != EMPTY_ARR_CID {
            return Err(actor_error!(illegal_state, "can't create over an existing actor"));
        }
        let store = rt.store().clone();
        Ok(Self {
            rt,
            slots: StateKamt::new_with_config(store, KAMT_CONFIG.clone()),
            nonce: 1,
            saved_state_root: None,
            bytecode: None,
            readonly: read_only,
        })
    }

    /// Load the actor from state.
    pub fn load(rt: &'r mut RT) -> Result<Self, ActorError>
    where
        RT::Blockstore: Clone,
    {
        let read_only = rt.read_only();
        let store = rt.store().clone();
        let state_root = rt.get_state_root()?;
        let state: State = store
            .get_cbor(&state_root)
            .context_code(ExitCode::USR_SERIALIZATION, "failed to decode state")?
            .context_code(ExitCode::USR_ILLEGAL_STATE, "state not in blockstore")?;

        Ok(Self {
            rt,
            slots: StateKamt::load_with_config(&state.contract_state, store, KAMT_CONFIG.clone())
                .context_code(ExitCode::USR_ILLEGAL_STATE, "state not in blockstore")?,
            nonce: state.nonce,
            saved_state_root: Some(state_root),
            bytecode: Some(state.bytecode),
            readonly: read_only,
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

    /// Send a message in "read-only" mode (for staticcall).
    pub fn send_read_only(
        &mut self,
        to: &Address,
        method: MethodNum,
        params: RawBytes,
    ) -> Result<RawBytes, ActorError> {
        self.flush()?;
        self.rt.send_read_only(to, method, params)
    }

    /// Generalized send
    pub fn send_generalized(
        &mut self,
        to: &Address,
        method: MethodNum,
        params: RawBytes,
        value: TokenAmount,
        gas_limit: Option<u64>,
        send_flags: SendFlags,
    ) -> Result<RawBytes, ActorError> {
        self.flush()?;
        let result = self.rt.send_generalized(to, method, params, value, gas_limit, send_flags)?;
        if !send_flags.read_only() {
            self.reload()?;
        }
        Ok(result)
    }

    /// Flush the actor state (bytecode, nonce, and slots).
    pub fn flush(&mut self) -> Result<(), ActorError> {
        if self.saved_state_root.is_some() {
            return Ok(());
        }

        if self.readonly {
            return Err(ActorError::forbidden("contract invocation is read only".to_string()));
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
        if self.readonly {
            return Ok(());
        }

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
        Ok(self.bytecode.as_ref().map(|k| load_bytecode(self.rt.store(), k)).transpose()?.flatten())
    }

    /// Set the bytecode.
    pub fn set_bytecode(&mut self, bytecode: &[u8]) -> Result<Cid, ActorError> {
        self.saved_state_root = None;
        if bytecode.len() > MAX_CODE_SIZE {
            return Err(ActorError::illegal_argument(format!(
                "EVM byte code length ({}) is exceeding the maximum allowed of {MAX_CODE_SIZE}",
                bytecode.len()
            )));
        } else if bytecode.first() == Some(&0xEF) {
            // Reject code starting with 0xEF, EIP-3541
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
    pub fn get_storage(&mut self, key: U256) -> Result<U256, StatusCode> {
        Ok(self
            .slots
            .get(&key)
            .map_err(|e| StatusCode::InternalError(e.to_string()))?
            .cloned()
            .unwrap_or_default())
    }

    /// Set value of a storage key.
    pub fn set_storage(&mut self, key: U256, value: U256) -> Result<(), StatusCode> {
        let changed = if value.is_zero() {
            self.slots.delete(&key).map(|v| v.is_some())
        } else {
            self.slots.set(key, value).map(|v| v != Some(value))
        }
        .map_err(|e| StatusCode::InternalError(e.to_string()))?;

        if changed {
            self.saved_state_root = None; // dirty.
        };
        Ok(())
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

pub fn load_bytecode<BS: Blockstore>(bs: &BS, cid: &Cid) -> Result<Option<Bytecode>, ActorError> {
    let bytecode = bs
        .get(cid)
        .context_code(ExitCode::USR_NOT_FOUND, "failed to read bytecode")?
        .expect("bytecode not in state tree");
    if bytecode.is_empty() {
        Ok(None)
    } else {
        Ok(Some(Bytecode::new(bytecode)))
    }
}
