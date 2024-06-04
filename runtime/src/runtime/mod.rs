// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::CborStore;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::randomness::RANDOMNESS_LENGTH;
use fvm_shared::version::NetworkVersion;
use fvm_shared::{ActorID, MethodNum, Response};
use serde::de::DeserializeOwned;
use serde::Serialize;

pub use self::actor_code::*;
pub use self::policy::*;
pub use self::randomness::DomainSeparationTag;
use crate::runtime::builtins::Type;
use crate::{actor_error, ActorError, SendError};

mod actor_code;
pub mod builtins;
pub mod policy;
mod randomness;

#[cfg(feature = "fil-actor")]
mod actor_blockstore;
#[cfg(feature = "fil-actor")]
pub mod fvm;
#[cfg(feature = "fil-actor")]
pub(crate) mod hash_algorithm;

pub(crate) mod empty;

use cid::multihash::Code;
pub use empty::EMPTY_ARR_CID;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::chainid::ChainID;
use fvm_shared::event::ActorEvent;
use fvm_shared::sys::SendFlags;
pub use vm_api::Primitives;

/// Runtime is the VM's internal runtime object.
/// this is everything that is accessible to actors, beyond parameters.
pub trait Runtime: Primitives + RuntimePolicy {
    type Blockstore: Blockstore;

    /// The network protocol version number at the current epoch.
    fn network_version(&self) -> NetworkVersion;

    /// Information related to the current message being executed.
    fn message(&self) -> &dyn MessageInfo;

    /// The current chain epoch number, corresponding to the epoch in which the message is executed.
    /// The genesis block has epoch zero.
    fn curr_epoch(&self) -> ChainEpoch;

    /// The ID for this chain.
    /// Filecoin chain IDs are usually in the Ethereum namespace, see: https://github.com/ethereum-lists/chains.
    fn chain_id(&self) -> ChainID;

    /// Validates the caller against some predicate.
    /// Exported actor methods must invoke at least one caller validation before returning.
    fn validate_immediate_caller_accept_any(&self) -> Result<(), ActorError>;
    fn validate_immediate_caller_is<'a, I>(&self, addresses: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Address>;
    /// Validates that the caller has a delegated address that is a member of
    /// one of the provided namespaces.
    /// Addresses must be of Protocol ID.
    fn validate_immediate_caller_namespace<I>(
        &self,
        namespace_manager_addresses: I,
    ) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = u64>;
    fn validate_immediate_caller_type<'a, I>(&self, types: I) -> Result<(), ActorError>
    where
        I: IntoIterator<Item = &'a Type>;

    /// The balance of the receiver.
    fn current_balance(&self) -> TokenAmount;

    /// The balance of an actor.
    fn actor_balance(&self, id: ActorID) -> Option<TokenAmount>;

    /// Resolves an address of any protocol to an ID address (via the Init actor's table).
    /// This allows resolution of externally-provided SECP, BLS, or actor addresses to the canonical form.
    /// If the argument is an ID address it is returned directly.
    fn resolve_address(&self, address: &Address) -> Option<ActorID>;

    /// Looks up the "delegated" address of an actor by ID, if any. Returns None if either the
    /// target actor doesn't exist, or doesn't have an f4 address.
    fn lookup_delegated_address(&self, id: ActorID) -> Option<Address>;

    /// Look up the code ID at an actor address.
    fn get_actor_code_cid(&self, id: &ActorID) -> Option<Cid>;

    /// Randomness returns a (pseudo)random byte array drawing from the latest
    /// ticket chain from a given epoch and incorporating requisite entropy.
    /// This randomness is fork dependant but also biasable because of this.
    fn get_randomness_from_tickets(
        &self,
        personalization: DomainSeparationTag,
        rand_epoch: ChainEpoch,
        entropy: &[u8],
    ) -> Result<[u8; RANDOMNESS_LENGTH], ActorError>;

    /// Randomness returns a (pseudo)random byte array drawing from the latest
    /// beacon from a given epoch and incorporating requisite entropy.
    /// This randomness is not tied to any fork of the chain, and is unbiasable.
    fn get_randomness_from_beacon(
        &self,
        personalization: DomainSeparationTag,
        rand_epoch: ChainEpoch,
        entropy: &[u8],
    ) -> Result<[u8; RANDOMNESS_LENGTH], ActorError>;

    /// Initializes the state object.
    /// This is only valid when the state has not yet been initialized.
    /// NOTE: we should also limit this to being invoked during the constructor method
    fn create<T: Serialize>(&self, obj: &T) -> Result<(), ActorError> {
        let root = self.get_state_root()?;
        if root != EMPTY_ARR_CID {
            return Err(
                actor_error!(illegal_state; "failed to create state; expected empty array CID, got: {}", root),
            );
        }
        let new_root = self.store().put_cbor(obj, Code::Blake2b256)
            .map_err(|e| actor_error!(illegal_argument; "failed to write actor state during creation: {}", e.to_string()))?;
        self.set_state_root(&new_root)?;
        Ok(())
    }

    /// Loads a readonly copy of the state of the receiver into the argument.
    fn state<T: DeserializeOwned>(&self) -> Result<T, ActorError> {
        Ok(self
            .store()
            .get_cbor(&self.get_state_root()?)
            .map_err(|_| actor_error!(illegal_argument; "failed to get actor for Readonly state"))?
            .expect("State does not exist for actor state root"))
    }

    /// Gets the state-root.
    fn get_state_root(&self) -> Result<Cid, ActorError>;

    /// Sets the state-root.
    fn set_state_root(&self, root: &Cid) -> Result<(), ActorError>;

    /// Loads a mutable copy of the state of the receiver, passes it to `f`,
    /// and after `f` completes puts the state object back to the store and sets it as
    /// the receiver's state root.
    ///
    /// During the call to `f`, execution is protected from side-effects, (including message send).
    ///
    /// Returns the result of `f`.
    fn transaction<S, RT, F>(&self, f: F) -> Result<RT, ActorError>
    where
        S: Serialize + DeserializeOwned,
        F: FnOnce(&mut S, &Self) -> Result<RT, ActorError>;

    /// Returns reference to blockstore
    fn store(&self) -> &Self::Blockstore;

    /// Sends a message to another actor, returning the exit code and return value envelope.
    /// If the invoked method does not return successfully, its state changes
    /// (and that of any messages it sent in turn) will be rolled back.
    fn send(
        &self,
        to: &Address,
        method: MethodNum,
        params: Option<IpldBlock>,
        value: TokenAmount,
        gas_limit: Option<u64>,
        flags: SendFlags,
    ) -> Result<Response, SendError>;

    /// Simplified version of [`Runtime::send`] that does not specify a gas limit, nor any send flags.
    fn send_simple(
        &self,
        to: &Address,
        method: MethodNum,
        params: Option<IpldBlock>,
        value: TokenAmount,
    ) -> Result<Response, SendError> {
        self.send(to, method, params, value, None, SendFlags::empty())
    }

    /// Computes an address for a new actor. The returned address is intended to uniquely refer to
    /// the actor even in the event of a chain re-org (whereas an ID-address might refer to a
    /// different actor after messages are re-ordered).
    /// Always an ActorExec address.
    fn new_actor_address(&self) -> Result<Address, ActorError>;

    /// Creates an actor with code `codeID`, an empty state, id `actor_id`, and an optional predictable address.
    /// May only be called by Init actor.
    fn create_actor(
        &self,
        code_id: Cid,
        actor_id: ActorID,
        predictable_address: Option<Address>,
    ) -> Result<(), ActorError>;

    /// Deletes the executing actor from the state tree. Fails if there is any unspent balance in
    /// the actor.
    ///
    /// May only be called by the actor itself.
    fn delete_actor(&self) -> Result<(), ActorError>;

    /// Returns whether the specified CodeCID belongs to a built-in actor.
    fn resolve_builtin_actor_type(&self, code_id: &Cid) -> Option<Type>;

    /// Returns the CodeCID for a built-in actor type. The kernel will abort
    /// if the supplied type is invalid.
    fn get_code_cid_for_type(&self, typ: Type) -> Cid;

    /// Returns the total token supply in circulation at the beginning of the current epoch.
    /// The circulating supply is the sum of:
    /// - rewards emitted by the reward actor,
    /// - funds vested from lock-ups in the genesis state,
    /// less the sum of:
    /// - funds burnt,
    /// - pledge collateral locked in storage miner actors (recorded in the storage power actor)
    /// - deal collateral locked by the storage market actor
    fn total_fil_circ_supply(&self) -> TokenAmount;

    /// ChargeGas charges specified amount of `gas` for execution.
    /// `name` provides information about gas charging point
    fn charge_gas(&self, name: &'static str, compute: i64);

    /// Returns the gas base fee (cost per unit) for the current epoch.
    fn base_fee(&self) -> TokenAmount;

    /// The gas still available for computation
    fn gas_available(&self) -> u64;

    /// The timestamp of the tipset at the current epoch (see curr_epoch), as UNIX seconds.
    fn tipset_timestamp(&self) -> u64;

    /// The CID of the tipset at the specified epoch.
    /// The epoch must satisfy: (curr_epoch - FINALITY) < epoch <= curr_epoch
    fn tipset_cid(&self, epoch: i64) -> Result<Cid, ActorError>;

    /// Emits an event denoting that something externally noteworthy has ocurred.
    fn emit_event(&self, event: &ActorEvent) -> Result<(), ActorError>;

    /// Returns true if the call is read_only.
    /// All state updates, including actor creation and balance transfers, are rejected in read_only calls.
    fn read_only(&self) -> bool;
}

/// Message information available to the actor about executing message.
pub trait MessageInfo {
    /// The nonce of the currently executing message.
    fn nonce(&self) -> u64;

    /// The address of the immediate calling actor. Always an ID-address.
    fn caller(&self) -> Address;

    /// The address of the origin of the current invocation. Always an ID-address
    fn origin(&self) -> Address;

    /// The address of the actor receiving the message. Always an ID-address.
    fn receiver(&self) -> Address;

    /// The value attached to the message being processed, implicitly
    /// added to current_balance() before method invocation.
    fn value_received(&self) -> TokenAmount;

    /// The message gas premium
    fn gas_premium(&self) -> TokenAmount;
}
