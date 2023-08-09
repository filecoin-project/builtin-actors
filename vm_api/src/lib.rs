use std::collections::BTreeMap;

use cid::Cid;
// TODO: drop the dependency on fil_actors_runtime and have a suitable replacement abstraction here
// https://github.com/filecoin-project/builtin-actors/issues/1344
pub use fil_actors_runtime::runtime::{builtins::Type, Primitives};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::{
    ipld_block::IpldBlock,
    tuple::{serde_tuple, Deserialize_tuple, Serialize_tuple},
};
use fvm_shared::{
    address::Address, clock::ChainEpoch, econ::TokenAmount, error::ExitCode, MethodNum,
};

mod error;
pub use error::*;

pub mod trace;
use trace::*;
pub mod util;

/// An abstract VM that is injected into integration tests
pub trait VM {
    /// Returns the underlying blockstore of the VM
    fn blockstore(&self) -> &dyn Blockstore;

    /// Get the current chain epoch
    fn epoch(&self) -> ChainEpoch;

    /// Sets the epoch to the specified value
    fn set_epoch(&self, epoch: ChainEpoch);

    /// Get information about an actor
    fn actor(&self, address: &Address) -> Option<ActorState>;

    /// Upsert an actor into the state tree
    fn set_actor(&self, key: &Address, a: ActorState);

    /// Get the balance of the specified actor
    fn balance(&self, address: &Address) -> TokenAmount;

    /// Get the ID for the specified address
    fn resolve_id_address(&self, address: &Address) -> Option<Address>;

    /// Send a message between the two specified actors
    fn execute_message(
        &self,
        from: &Address,
        to: &Address,
        value: &TokenAmount,
        method: MethodNum,
        params: Option<IpldBlock>,
    ) -> Result<MessageResult, VMError>;

    /// Send a message without charging gas
    fn execute_message_implicit(
        &self,
        from: &Address,
        to: &Address,
        value: &TokenAmount,
        method: MethodNum,
        params: Option<IpldBlock>,
    ) -> Result<MessageResult, VMError>;

    /// Take all the invocations that have been made since the last call to this method
    fn take_invocations(&self) -> Vec<InvocationTrace>;

    /// Set the circulating supply constant for the network
    fn set_circulating_supply(&self, supply: TokenAmount);

    /// Get the circulating supply constant for the network
    fn circulating_supply(&self) -> TokenAmount;

    /// Provides access to VM primitives
    fn primitives(&self) -> &dyn Primitives;

    /// Return a map of actor code CIDs to their corresponding types
    fn actor_manifest(&self) -> BTreeMap<Cid, Type>;

    /// Return the root of the state tree
    fn state_root(&self) -> Cid;
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MessageResult {
    pub code: ExitCode,
    pub message: String,
    pub ret: Option<IpldBlock>,
}

// Duplicates an internal FVM type (fvm::state_tree::ActorState) that cannot be depended on here
#[derive(Serialize_tuple, Deserialize_tuple, Clone, PartialEq, Eq, Debug)]
pub struct ActorState {
    /// Link to code for the actor.
    pub code: Cid,
    /// Link to the state of the actor.
    pub state: Cid,
    /// Sequence of the actor.
    pub sequence: u64,
    /// Tokens available to the actor.
    pub balance: TokenAmount,
    /// The actor's "delegated" address, if assigned.
    ///
    /// This field is set on actor creation and never modified.
    pub delegated_address: Option<Address>,
}

pub fn new_actor(
    code: Cid,
    head: Cid,
    call_seq_num: u64,
    balance: TokenAmount,
    predictable_address: Option<Address>,
) -> ActorState {
    ActorState {
        code,
        state: head,
        sequence: call_seq_num,
        balance,
        delegated_address: predictable_address,
    }
}
