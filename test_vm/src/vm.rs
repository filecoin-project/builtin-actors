/*!
 * The VM module is replicated in this code tree temporarily. This is the high-level abstract interface
 * for a virtual-machine that can execute Filecoin WASM actors. It defines the high-level virtual-machine
 * interface, associated error and trace types and an interface to inject/override the behaviour of
 * certain primitives for the purpose of running tests.
 *
 * TODO(alexytsu): It should eventually be moved to an external location so that it can be shared
 * with the anorth/fvm-workbench implementation
 */
use std::{collections::BTreeMap, error::Error, fmt};

use cid::Cid;
use fil_actors_runtime::runtime::{builtins::Type, Primitives};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::{
    ipld_block::IpldBlock,
    tuple::{serde_tuple, Deserialize_tuple, Serialize_tuple},
};
use fvm_shared::{address::Address, clock::ChainEpoch, econ::TokenAmount, MethodNum};

use crate::{trace::InvocationTrace, MessageResult};

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

    // TODO: set circulating supply
    fn circulating_supply(&self) -> TokenAmount;

    /// Provides access to VM primitives
    fn primitives(&self) -> &dyn Primitives;

    fn actor_manifest(&self) -> BTreeMap<Cid, Type>;

    fn state_root(&self) -> Cid;
}

#[derive(Debug)]
pub struct VMError {
    msg: String,
}

impl fmt::Display for VMError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

impl Error for VMError {
    fn description(&self) -> &str {
        &self.msg
    }
}

impl From<fvm_ipld_hamt::Error> for VMError {
    fn from(h_err: fvm_ipld_hamt::Error) -> Self {
        vm_err(h_err.to_string().as_str())
    }
}

pub fn vm_err(msg: &str) -> VMError {
    VMError { msg: msg.to_string() }
}

#[derive(Serialize_tuple, Deserialize_tuple, Clone, PartialEq, Eq, Debug)]
pub struct ActorState {
    pub code: Cid,
    pub state: Cid,
    pub call_seq: u64,
    pub balance: TokenAmount,
    pub predictable_address: Option<Address>,
}

pub fn actor(
    code: Cid,
    head: Cid,
    call_seq_num: u64,
    balance: TokenAmount,
    predictable_address: Option<Address>,
) -> ActorState {
    ActorState { code, state: head, call_seq: call_seq_num, balance, predictable_address }
}
