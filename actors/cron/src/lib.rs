// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{actor_error, cbor, ActorError, SYSTEM_ACTOR_ADDR};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::econ::TokenAmount;

use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;

pub use self::state::{Entry, State};

mod state;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

// * Updated to specs-actors commit: 845089a6d2580e46055c24415a6c32ee688e5186 (v3.0.0)

/// Cron actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    EpochTick = 2,
}

/// Constructor parameters for Cron actor, contains entries
/// of actors and methods to call on each epoch
#[derive(Default, Debug, Serialize_tuple, Deserialize_tuple)]
pub struct ConstructorParams {
    /// Entries is a set of actors (and corresponding methods) to call during EpochTick.
    pub entries: Vec<Entry>,
}

/// Cron actor
pub struct Actor;
impl Actor {
    /// Constructor for Cron actor
    fn constructor<BS, RT>(rt: &mut RT, params: ConstructorParams) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_is(std::iter::once(&*SYSTEM_ACTOR_ADDR))?;
        rt.create(&State { entries: params.entries })?;
        Ok(())
    }
    /// Executes built-in periodic actions, run at every Epoch.
    /// epoch_tick(r) is called after all other messages in the epoch have been applied.
    /// This can be seen as an implicit last message.
    fn epoch_tick<BS, RT>(rt: &mut RT) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_is(std::iter::once(&*SYSTEM_ACTOR_ADDR))?;

        let st: State = rt.state()?;
        for entry in st.entries {
            // Intentionally ignore any error when calling cron methods
            let res = rt.send(
                entry.receiver,
                entry.method_num,
                RawBytes::default(),
                TokenAmount::from(0u8),
            );
            if let Err(e) = res {
                log::error!(
                    "cron failed to send entry to {}, send error code {}",
                    entry.receiver,
                    e
                );
            }
        }
        Ok(())
    }
}

impl ActorCode for Actor {
    fn invoke_method<BS, RT>(
        rt: &mut RT,
        method: MethodNum,
        params: &RawBytes,
    ) -> Result<RawBytes, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        match FromPrimitive::from_u64(method) {
            Some(Method::Constructor) => {
                Self::constructor(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::EpochTick) => {
                Self::epoch_tick(rt)?;
                Ok(RawBytes::default())
            }
            None => Err(actor_error!(SysErrInvalidMethod; "Invalid method")),
        }
    }
}
