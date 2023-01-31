#![allow(dead_code)]

use std::borrow::Cow;

use fil_actors_runtime::{actor_error, runtime::EMPTY_ARR_CID, AsActorError, EAM_ACTOR_ID};
use fvm_ipld_blockstore::Block;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_ipld_encoding::CborStore;
use fvm_ipld_kamt::HashedKey;
use fvm_shared::{
    address::{Address, Payload},
    crypto::hash::SupportedHashes,
    econ::TokenAmount,
    error::{ErrorNumber, ExitCode},
    sys::SendFlags,
    MethodNum, Response, IPLD_RAW, METHOD_SEND,
};
use multihash::Code;
use once_cell::unsync::OnceCell;

use crate::state::{State, Tombstone};
use crate::BytecodeHash;

use super::{address::EthAddress, Bytecode};

use {
    crate::interpreter::U256,
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

#[derive(Clone, Copy)]
pub struct EvmBytecode {
    /// CID of the contract
    pub cid: Cid,
    /// Keccak256 hash of the contract
    pub evm_hash: BytecodeHash,
}

impl EvmBytecode {
    fn new(cid: Cid, evm_hash: BytecodeHash) -> Self {
        Self { cid, evm_hash }
    }
}

/// Platform Abstraction Layer
/// that bridges the FVM world to EVM world
pub struct System<'r, RT: Runtime> {
    pub rt: &'r mut RT,

    /// The current bytecode. This is usually only "none" when the actor is first constructed.
    /// (blake2b256(ipld_raw(bytecode)), keccak256(bytecode))
    bytecode: Option<EvmBytecode>,
    /// The contract's EVM storage slots.
    slots: StateKamt<RT::Blockstore>,
    /// The contracts "nonce" (incremented when creating new actors).
    nonce: u64,
    /// The last saved state root. None if the current state hasn't been saved yet.
    saved_state_root: Option<Cid>,
    /// Read Only context (staticcall)
    pub readonly: bool,
    /// Randomness taken from the current epoch of chain randomness
    randomness: OnceCell<[u8; 32]>,

    /// This is "some" if the actor is currently a "zombie". I.e., it has selfdestructed, but the
    /// current message is still executing. `System` cannot load a contracts state with a
    tombstone: Option<Tombstone>,
}

impl<'r, RT: Runtime> System<'r, RT> {
    pub(crate) fn new(rt: &'r mut RT, readonly: bool) -> Self
    where
        RT::Blockstore: Clone,
    {
        let store = rt.store().clone();
        Self {
            rt,
            slots: StateKamt::new_with_config(store, KAMT_CONFIG.clone()),
            nonce: 1,
            saved_state_root: None,
            bytecode: None,
            readonly,
            randomness: OnceCell::new(),
            tombstone: None,
        }
    }

    /// Resurrect the contract. This will return a new empty contract if, and only if, the contract
    /// is "dead".
    pub fn resurrect(rt: &'r mut RT) -> Result<Self, ActorError>
    where
        RT::Blockstore: Clone,
    {
        let read_only = rt.read_only();
        let state_root = rt.get_state_root()?;
        // Check the tombstone.
        let state: State = rt
            .store()
            .get_cbor(&state_root)
            .context_code(ExitCode::USR_SERIALIZATION, "failed to decode state")?
            .context_code(ExitCode::USR_ILLEGAL_STATE, "state not in blockstore")?;
        if !crate::is_dead(rt, &state) {
            return Err(actor_error!(forbidden, "can only resurrect a dead contract"));
        }

        return Ok(Self::new(rt, read_only));
    }

    /// Create the contract. This will return a new empty contract if, and only if, the contract
    /// doesn't have any state.
    pub fn create(rt: &'r mut RT) -> Result<Self, ActorError>
    where
        RT::Blockstore: Clone,
    {
        let read_only = rt.read_only();
        let state_root = rt.get_state_root()?;
        if state_root != EMPTY_ARR_CID {
            return Err(actor_error!(illegal_state, "can't create over an existing actor"));
        }
        return Ok(Self::new(rt, read_only));
    }

    /// Load the actor from state.
    pub fn load(rt: &'r mut RT) -> Result<Self, ActorError>
    where
        RT::Blockstore: Clone,
    {
        let store = rt.store().clone();
        let state_root = rt.get_state_root()?;
        let state: State = store
            .get_cbor(&state_root)
            .context_code(ExitCode::USR_SERIALIZATION, "failed to decode state")?
            .context_code(ExitCode::USR_ILLEGAL_STATE, "state not in blockstore")?;

        if crate::is_dead(rt, &state) {
            // If we're "dead", return an empty read-only contract. The code will be empty, so
            // nothing can happen anyways.
            return Ok(Self::new(rt, true));
        }

        let read_only = rt.read_only();

        Ok(Self {
            rt,
            slots: StateKamt::load_with_config(&state.contract_state, store, KAMT_CONFIG.clone())
                .context_code(ExitCode::USR_ILLEGAL_STATE, "state not in blockstore")?,
            nonce: state.nonce,
            saved_state_root: Some(state_root),
            bytecode: Some(EvmBytecode::new(state.bytecode, state.bytecode_hash)),
            readonly: read_only,
            randomness: OnceCell::new(),
            tombstone: state.tombstone,
        })
    }

    pub fn increment_nonce(&mut self) -> u64 {
        self.saved_state_root = None;
        let nonce = self.nonce;
        self.nonce = self.nonce.checked_add(1).unwrap();
        nonce
    }

    /// Transfers funds to the receiver. This doesn't bother saving/reloading state.
    pub fn transfer(&mut self, to: &Address, value: TokenAmount) -> Result<(), ActorError> {
        self.rt.send(to, METHOD_SEND, None, value)?;
        Ok(())
    }

    /// Generalized send
    pub fn send(
        &mut self,
        to: &Address,
        method: MethodNum,
        params: Option<IpldBlock>,
        value: TokenAmount,
        gas_limit: Option<u64>,
        send_flags: SendFlags,
    ) -> Result<Option<IpldBlock>, ActorError> {
        let result = self.send_raw(to, method, params, value, gas_limit, send_flags)?.map_err(|err| {
            actor_error!(unspecified; "send syscall to {to} on method {method} failed: {}", err)
        })?;

        // Don't bother reloading on abort, just return the error.
        if !result.exit_code.is_success() {
            return Err(ActorError::checked(
                result.exit_code,
                format!("failed to call {to} on method {method}"),
                result.return_data,
            ));
        }

        Ok(result.return_data)
    }

    /// Send, but get back the raw syscall error failure without interpreting it as an actor error.
    /// This method has a really funky return type because:
    /// 1. It can fail with an outer "actor error". In that case, the EVM is expected to abort with
    ///    the specified exit code.
    /// 2. It can fail with an inner syscall error.
    /// 3. It can successfully call into the other actor, and return a response with a non-zero exit code.
    pub fn send_raw(
        &mut self,
        to: &Address,
        method: MethodNum,
        params: Option<IpldBlock>,
        value: TokenAmount,
        gas_limit: Option<u64>,
        send_flags: SendFlags,
    ) -> Result<Result<Response, ErrorNumber>, ActorError> {
        self.flush()?;
        let result = self.rt.send_generalized(to, method, params, value, gas_limit, send_flags);

        // Reload on success, and only on success.
        match &result {
            Ok(r) if r.exit_code.is_success() => self.reload()?,
            _ => {}
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

        let EvmBytecode { cid, evm_hash } = match self.bytecode {
            Some(cid) => cid,
            // set empty bytecode hashes
            None => self.set_bytecode(&[])?,
        };
        let new_root = self
            .rt
            .store()
            .put_cbor(
                &State {
                    bytecode: cid,
                    bytecode_hash: evm_hash,
                    contract_state: self.slots.flush().context_code(
                        ExitCode::USR_ILLEGAL_STATE,
                        "failed to flush contract state",
                    )?,
                    nonce: self.nonce,
                    tombstone: self.tombstone,
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
        self.bytecode = Some(EvmBytecode::new(state.bytecode, state.bytecode_hash));
        Ok(())
    }

    /// Get the bytecode, if any.
    pub fn get_bytecode(&self) -> Option<Cid> {
        self.bytecode.as_ref().map(|b| b.cid)
    }

    /// Set the bytecode.
    pub fn set_bytecode(&mut self, bytecode: &[u8]) -> Result<EvmBytecode, ActorError> {
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

        let code_hash = self.rt.hash(SupportedHashes::Keccak256, bytecode)[..]
            .try_into()
            .context_code(ExitCode::USR_ASSERTION_FAILED, "expected a 32byte digest")?;

        let cid = self
            .rt
            .store()
            .put(Code::Blake2b256, &Block::new(IPLD_RAW, bytecode))
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to write bytecode")?;
        let bytecode = EvmBytecode::new(cid, code_hash);
        self.bytecode = Some(bytecode);
        Ok(bytecode)
    }

    /// Get value of a storage key.
    pub fn get_storage(&mut self, key: U256) -> Result<U256, ActorError> {
        Ok(self
            .slots
            .get(&key)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to clear storage slot")?
            .cloned()
            .unwrap_or_default())
    }

    /// Set value of a storage key.
    pub fn set_storage(&mut self, key: U256, value: U256) -> Result<(), ActorError> {
        let changed = if value.is_zero() {
            self.slots
                .delete(&key)
                .map(|v| v.is_some())
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to clear storage slot")?
        } else {
            self.slots
                .set(key, value)
                .map(|v| v != Some(value))
                .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to update storage slot")?
        };

        if changed {
            self.saved_state_root = None; // dirty.
        };
        Ok(())
    }

    /// Resolve the address to the ethereum equivalent, if possible.
    ///
    /// - Eth f4 maps directly to an Eth address.
    /// - f3, f2, and f1, addresses will resolve to ID address then...
    /// - Attempt to lookup Eth f4 address from ID address.
    /// - Otherwise encode ID address into Eth address (0xff....\<id>)
    pub fn resolve_ethereum_address(&self, addr: &Address) -> Result<EthAddress, ActorError> {
        // Short-circuit if we already have an EVM actor.
        match addr.payload() {
            Payload::Delegated(delegated) if delegated.namespace() == EAM_ACTOR_ID => {
                let subaddr: [u8; 20] = delegated
                    .subaddress()
                    .try_into()
                    .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                        format!("invalid ethereum address length: {addr}")
                    })?;
                return Ok(EthAddress(subaddr));
            }
            _ => {}
        }

        // Otherwise, resolve to an ID address.
        let actor_id = self.rt.resolve_address(addr).context_code(
            ExitCode::USR_ILLEGAL_STATE,
            "non-ethereum address {addr} cannot be resolved to an ID address",
        )?;

        // Then attempt to resolve back into an EVM address.
        match self.rt.lookup_delegated_address(actor_id).map(|a| a.into_payload()) {
            Some(Payload::Delegated(delegated)) if delegated.namespace() == EAM_ACTOR_ID => {
                let subaddr: [u8; 20] = delegated.subaddress().try_into().context_code(
                    ExitCode::USR_ILLEGAL_STATE,
                    "invalid ethereum address length: {addr}",
                )?;
                Ok(EthAddress(subaddr))
            }
            // But use an EVM address as the fallback.
            _ => Ok(EthAddress::from_id(actor_id)),
        }
    }

    /// Gets the cached EVM randomness seed of the current epoch
    pub fn get_randomness(&mut self) -> Result<&[u8; 32], ActorError> {
        const ENTROPY: &[u8] = b"prevrandao";
        self.randomness.get_or_try_init(|| {
            // get randomness from current beacon epoch with entropy of "prevrandao"
            self.rt.get_randomness_from_beacon(
                fil_actors_runtime::runtime::DomainSeparationTag::EvmPrevRandao,
                self.rt.curr_epoch(),
                ENTROPY,
            )
        })
    }

    /// Mark ourselves as "selfdestructed".
    pub fn mark_selfdestructed(&mut self) {
        self.saved_state_root = None;
        self.tombstone = Some(crate::current_tombstone(self.rt));
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
