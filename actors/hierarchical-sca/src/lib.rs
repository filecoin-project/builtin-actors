// Copyright 2019-2022 ConsensusLab
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    actor_error, cbor, ActorDowncast, ActorError, BURNT_FUNDS_ACTOR_ADDR, SYSTEM_ACTOR_ADDR,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::actor::builtin::Type;
use fvm_shared::bigint::Zero;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::METHOD_SEND;
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use std::collections::HashMap;

pub use self::checkpoint::{Checkpoint, CrossMsgMeta};
pub use self::state::*;
pub use self::subnet::*;
pub use self::types::*;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

pub mod checkpoint;
#[doc(hidden)]
pub mod ext;
mod state;
pub mod subnet;
mod types;

/// SCA actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    /// Constructor for Storage Power Actor
    Constructor = METHOD_CONSTRUCTOR,
    Register = 2,
    AddStake = 3,
    ReleaseStake = 4,
    Kill = 5,
    CommitChildCheckpoint = 6,
}

/// Subnet Coordinator Actor
pub struct Actor;
impl Actor {
    /// Constructor for SCA actor
    fn constructor<BS, RT>(rt: &mut RT, params: ConstructorParams) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_is(std::iter::once(&*SYSTEM_ACTOR_ADDR))?;

        let st = State::new(rt.store(), params).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "Failed to create SCA actor state")
        })?;
        rt.create(&st)?;
        Ok(())
    }

    /// Register is called by subnet actors to put the required collateral
    /// and register the subnet to the hierarchy.
    fn register<BS, RT>(rt: &mut RT) -> Result<SubnetIDParam, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Subnet))?;
        let subnet_addr = rt.message().caller();
        let mut shid = subnet::SubnetID::default();
        rt.transaction(|st: &mut State, rt| {
            shid = SubnetID::new(&st.network_name, subnet_addr);
            let sub = st.get_subnet(rt.store(), &shid).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load subnet")
            })?;
            match sub {
                Some(_) => {
                    return Err(actor_error!(
                        illegal_argument,
                        "subnet with id {} already registered",
                        shid
                    ))
                }
                None => {
                    st.register_subnet(rt, &shid).map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_ARGUMENT,
                            "Failed to register subnet",
                        )
                    })?;
                }
            }

            Ok(())
        })?;

        Ok(SubnetIDParam { id: shid.to_string() })
    }

    /// Add stake adds stake to the collateral of a subnet.
    fn add_stake<BS, RT>(rt: &mut RT) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Subnet))?;
        let subnet_addr = rt.message().caller();

        let val = rt.message().value_received();
        if val <= TokenAmount::zero() {
            return Err(actor_error!(illegal_argument, "no stake to add"));
        }

        rt.transaction(|st: &mut State, rt| {
            let shid = SubnetID::new(&st.network_name, subnet_addr);
            let sub = st.get_subnet(rt.store(), &shid).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load subnet")
            })?;
            match sub {
                Some(mut sub) => {
                    sub.add_stake(rt, st, &val).map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            "Failed to add stake to subnet",
                        )
                    })?;
                }
                None => {
                    return Err(actor_error!(
                        illegal_argument,
                        "subnet with id {} not registered",
                        shid
                    ))
                }
            }

            Ok(())
        })?;

        Ok(())
    }

    /// Release stake recovers some collateral of the subnet
    fn release_stake<BS, RT>(rt: &mut RT, params: FundParams) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Subnet))?;
        let subnet_addr = rt.message().caller();

        if params.value <= TokenAmount::zero() {
            return Err(actor_error!(illegal_argument, "no funds to release in params"));
        }
        let send_val = params.value.clone();

        rt.transaction(|st: &mut State, rt| {
            let shid = SubnetID::new(&st.network_name, subnet_addr);
            let sub = st.get_subnet(rt.store(), &shid).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load subnet")
            })?;
            match sub {
                Some(mut sub) => {
                    if sub.stake < params.value {
                        return Err(actor_error!(
                            illegal_state,
                            "subnet actor not allowed to release so many funds"
                        ));
                    }
                    // sanity-check: see if the actor has enough balance.
                    if rt.current_balance() < params.value{
                        return Err(actor_error!(
                            illegal_state,
                            "something went really wrong! the actor doesn't have enough balance to release"
                        ));
                    }
                     sub.add_stake(rt, st, &-params.value).map_err(|e| {
                         e.downcast_default(
                             ExitCode::USR_ILLEGAL_STATE,
                             "Failed to add stake to subnet",
                         )
                    })?;
                }
                None => {
                    return Err(actor_error!(
                        illegal_argument,
                        "subnet with id {} not registered",
                        shid
                    ))
                }
            }

            Ok(())
        })?;

        rt.send(subnet_addr, METHOD_SEND, RawBytes::default(), send_val.clone())?;
        Ok(())
    }

    /// Kill propagates the kill signal from a subnet actor to unregister it from th
    /// hierarchy.
    fn kill<BS, RT>(rt: &mut RT) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Subnet))?;
        let subnet_addr = rt.message().caller();
        let mut send_val = TokenAmount::zero();

        rt.transaction(|st: &mut State, rt| {
            let shid = SubnetID::new(&st.network_name, subnet_addr);
            let sub = st.get_subnet(rt.store(), &shid).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load subnet")
            })?;
            match sub {
                Some(sub) => {
                    if rt.current_balance() < sub.stake {
                        return Err(actor_error!(
                            illegal_state,
                            "something went really wrong! the actor doesn't have enough balance to release"
                        ));
                    }
                    if sub.circ_supply > TokenAmount::zero() {
                        return Err(actor_error!(
                            illegal_state,
                            "cannot kill a subnet that still holds user funds in its circ. supply"
                        ));
                    }
                    send_val = sub.stake;
                    // delete subnet
                    st.rm_subnet(rt.store(), &shid).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load subnet")
            })?;
                }
                None => {
                    return Err(actor_error!(
                        illegal_argument,
                        "subnet with id {} not registered",
                        shid
                    ))
                }
            }

            Ok(())
        })?;

        rt.send(subnet_addr, METHOD_SEND, RawBytes::default(), send_val.clone())?;
        Ok(())
    }

    /// CommitChildCheck propagates the commitment of a checkpoint from a child subnet,
    /// process the cross-messages directed to the subnet, and propagates the corresponding
    /// once further.
    fn commit_child_check<BS, RT>(rt: &mut RT, params: Checkpoint) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.validate_immediate_caller_type(std::iter::once(&Type::Subnet))?;
        let subnet_addr = rt.message().caller();
        let commit = params;

        // check if the checkpoint belongs to the subnet
        if subnet_addr != commit.source().subnet_actor() {
            return Err(actor_error!(
                illegal_argument,
                "source in checkpoint doesn't belong to subnet"
            ));
        }

        let mut burn_value = TokenAmount::zero();
        rt.transaction(|st: &mut State, rt| {
            let shid = SubnetID::new(&st.network_name, subnet_addr);
            let sub = st.get_subnet(rt.store(), &shid).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load subnet")
            })?;
            match sub {
                Some(mut sub) => {
                    // check if subnet active
                    if sub.status != Status::Active {
                        return Err(actor_error!(
                            illegal_state,
                            "can't commit checkpoint for an inactive subnet"
                        ));
                    }

                    // get window checkpoint being populated to include child info
                    let mut ch =
                        st.get_window_checkpoint(rt.store(), rt.curr_epoch()).map_err(|e| {
                            e.downcast_default(
                                ExitCode::USR_ILLEGAL_STATE,
                                "failed to get current epoch checkpoint",
                            )
                        })?;

                    // if this is not the first checkpoint we need to perform some
                    // additional verifications.
                    if sub.prev_checkpoint != Checkpoint::default() {
                        if sub.prev_checkpoint.epoch() > commit.epoch() {
                            return Err(actor_error!(
                                illegal_argument,
                                "checkpoint being committed belongs to the past"
                            ));
                        }
                        // check that the previous cid is consistent with the previous one
                        if sub.prev_checkpoint.cid() != commit.prev_check() {
                            return Err(actor_error!(
                                illegal_argument,
                                "previous checkpoint not consistente with previous one"
                            ));
                        }
                    }

                    // process and commit the checkpoint
                    // apply check messages
                    let ap_msgs: HashMap<SubnetID, Vec<&CrossMsgMeta>>;
                    (burn_value, ap_msgs) =
                        st.apply_check_msgs(rt.store(), &mut sub, &commit).map_err(|e| {
                            e.downcast_default(
                                ExitCode::USR_ILLEGAL_STATE,
                                "error applying check messages",
                            )
                        })?;
                    // aggregate message metas in checkpoint
                    st.agg_child_msgmeta(rt.store(), &mut ch, ap_msgs).map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            "error aggregating child msgmeta",
                        )
                    })?;
                    // append new checkpoint to the list of childs
                    ch.add_child_check(&commit).map_err(|e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_ARGUMENT,
                            "error adding child checkpoint",
                        )
                    })?;
                    // flush checkpoint
                    st.flush_checkpoint(rt.store(), &ch).map_err(|e| {
                        e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "error flushing checkpoint")
                    })?;

                    // update prev_check for child
                    sub.prev_checkpoint = commit;
                    // flush subnet
                    st.flush_subnet(rt.store(), &sub).map_err(|e| {
                        e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "error flushing subnet")
                    })?;
                }
                None => {
                    return Err(actor_error!(
                        illegal_argument,
                        "subnet with id {} not registered",
                        shid
                    ))
                }
            }

            Ok(())
        })?;

        if burn_value > TokenAmount::zero() {
            rt.send(*BURNT_FUNDS_ACTOR_ADDR, METHOD_SEND, RawBytes::default(), burn_value.clone())?;
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
            Some(Method::Register) => {
                let res = Self::register(rt)?;
                Ok(RawBytes::serialize(res)?)
            }
            Some(Method::AddStake) => {
                Self::add_stake(rt)?;
                Ok(RawBytes::default())
            }
            Some(Method::ReleaseStake) => {
                Self::release_stake(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::Kill) => {
                Self::kill(rt)?;
                Ok(RawBytes::default())
            }
            Some(Method::CommitChildCheckpoint) => {
                Self::commit_child_check(rt, cbor::deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            None => Err(actor_error!(unhandled_message; "Invalid method")),
        }
    }
}
