use fil_fungible_token::token;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::Cbor;
use fvm_shared::address::Address;
use fvm_shared::error::ExitCode;

use fil_actors_runtime::{ActorError, AsActorError};

#[derive(Serialize_tuple, Deserialize_tuple)]
pub struct State {
    pub registry: Address,
    pub token: token::state::TokenState,
}

impl State {
    pub fn new<BS: Blockstore>(store: &BS, registry: Address) -> Result<State, ActorError> {
        let token_state = token::state::TokenState::new(store)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to create token state")?;
        Ok(State { registry, token: token_state })
    }
}

impl Cbor for State {}
