// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    actor_error, resolve_to_id_addr, wasm_trampoline, ActorDowncast, ActorError, Array,
};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::RawBytes;
use fvm_shared::actor::builtin::Type;
use fvm_shared::address::Address;
use fvm_shared::bigint::{BigInt, Sign};
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::{MethodNum, METHOD_CONSTRUCTOR, METHOD_SEND};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;

pub use self::state::{LaneState, Merge, State};
pub use self::types::*;

wasm_trampoline!(Actor);

mod state;
mod types;

const ERR_USR_CHANNEL_STATE_UPDATE_AFTER_SETTLED: ExitCode = ExitCode::new(32);

// * Updated to specs-actors commit: f47f461b0588e9f0c20c999f6f129c85d669a7aa (v3.0.2)

/// Payment Channel actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    UpdateChannelState = 2,
    Settle = 3,
    Collect = 4,
}

/// Payment Channel actor
pub struct Actor;
impl Actor {
    /// Constructor for Payment channel actor
    pub fn constructor<BS, RT>(rt: &mut RT, params: ConstructorParams) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        // Only InitActor can create a payment channel actor. It creates the actor on
        // behalf of the payer/payee.
        rt.validate_immediate_caller_type(std::iter::once(&Type::Init))?;

        // Check both parties are capable of signing vouchers
        let to = Self::resolve_account(rt, &params.to)?;

        let from = Self::resolve_account(rt, &params.from)?;

        let empty_arr_cid =
            Array::<(), _>::new_with_bit_width(rt.store(), LANE_STATES_AMT_BITWIDTH)
                .flush()
                .map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to create empty AMT")
                })?;

        rt.create(&State::new(from, to, empty_arr_cid))?;
        Ok(())
    }

    /// Resolves an address to a canonical ID address and requires it to address an account actor.
    fn resolve_account<BS, RT>(rt: &mut RT, raw: &Address) -> Result<Address, ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        let resolved = resolve_to_id_addr(rt, raw).map_err(|e| {
            e.downcast_default(
                ExitCode::USR_ILLEGAL_STATE,
                format!("failed to resolve address {}", raw),
            )
        })?;

        let code_cid = rt.get_actor_code_cid(&resolved).ok_or_else(|| {
            actor_error!(USR_ILLEGAL_ARGUMENT, "no code for address {}", resolved)
        })?;

        let typ = rt.resolve_builtin_actor_type(&code_cid);

        if typ != Some(Type::Account) {
            Err(actor_error!(
                USR_FORBIDDEN,
                "actor {} must be an account, was {} ({:?})",
                raw,
                code_cid,
                typ
            ))
        } else {
            Ok(resolved)
        }
    }

    pub fn update_channel_state<BS, RT>(
        rt: &mut RT,
        params: UpdateChannelStateParams,
    ) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        let st: State = rt.state()?;

        rt.validate_immediate_caller_is([st.from, st.to].iter())?;
        let signer = if rt.message().caller() == st.from {
            st.to
        } else {
            st.from
        };
        let sv = params.sv;

        // Pull signature from signed voucher
        let sig = sv
            .signature
            .as_ref()
            .ok_or_else(|| actor_error!(USR_ILLEGAL_ARGUMENT, "voucher has no signature"))?;

        if st.settling_at != 0 && rt.curr_epoch() >= st.settling_at {
            return Err(ActorError::new(
                ERR_USR_CHANNEL_STATE_UPDATE_AFTER_SETTLED,
                "no vouchers can be processed after settling at epoch".to_string(),
            ));
        }

        if params.secret.len() > MAX_SECRET_SIZE {
            return Err(actor_error!(
                USR_ILLEGAL_ARGUMENT,
                "secret must be at most 256 bytes long"
            ));
        }

        // Generate unsigned bytes
        let sv_bz = sv
            .signing_bytes()
            .map_err(|e| ActorError::from(e).wrap("failed to serialized SignedVoucher"))?;

        // Validate signature
        rt.verify_signature(sig, &signer, &sv_bz).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_ARGUMENT, "voucher signature invalid")
        })?;

        let pch_addr = rt.message().receiver();
        let svpch_id_addr = rt.resolve_address(&sv.channel_addr).ok_or_else(|| {
            actor_error!(
                USR_ILLEGAL_ARGUMENT,
                "voucher payment channel address {} does not resolve to an ID address",
                sv.channel_addr
            )
        })?;
        if pch_addr != svpch_id_addr {
            return Err(actor_error!(USR_ILLEGAL_ARGUMENT;
                    "voucher payment channel address {} does not match receiver {}",
                    svpch_id_addr, pch_addr));
        }

        if rt.curr_epoch() < sv.time_lock_min {
            return Err(actor_error!(USR_ILLEGAL_ARGUMENT; "cannot use this voucher yet"));
        }

        if sv.time_lock_max != 0 && rt.curr_epoch() > sv.time_lock_max {
            return Err(actor_error!(USR_ILLEGAL_ARGUMENT; "this voucher has expired"));
        }

        if sv.amount.sign() == Sign::Minus {
            return Err(actor_error!(USR_ILLEGAL_ARGUMENT;
                    "voucher amount must be non-negative, was {}", sv.amount));
        }

        if !sv.secret_pre_image.is_empty() {
            let hashed_secret: &[u8] = &rt.hash_blake2b(&params.secret);
            if hashed_secret != sv.secret_pre_image.as_slice() {
                return Err(actor_error!(USR_ILLEGAL_ARGUMENT; "incorrect secret"));
            }
        }

        if let Some(extra) = &sv.extra {
            rt.send(
                extra.actor,
                extra.method,
                RawBytes::serialize(&extra.data)?,
                TokenAmount::from(0u8),
            )
            .map_err(|e| e.wrap("spend voucher verification failed"))?;
        }

        rt.transaction(|st: &mut State, rt| {
            let mut l_states = Array::load(&st.lane_states, rt.store()).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load lane states")
            })?;

            // Find the voucher lane, create and insert it in sorted order if necessary.
            let lane_id = sv.lane;
            let lane_state = find_lane(&l_states, lane_id)?;

            let mut lane_state = if let Some(state) = lane_state {
                if state.nonce >= sv.nonce {
                    return Err(actor_error!(USR_ILLEGAL_ARGUMENT;
                        "voucher has an outdated nonce, existing: {}, voucher: {}, cannot redeem",
                        state.nonce, sv.nonce));
                }
                state.clone()
            } else {
                LaneState::default()
            };

            // The next section actually calculates the payment amounts to update
            // the payment channel state
            // 1. (optional) sum already redeemed value of all merging lanes
            let mut redeemed_from_others = BigInt::default();
            for merge in sv.merges {
                if merge.lane == sv.lane {
                    return Err(actor_error!(USR_ILLEGAL_ARGUMENT;
                        "voucher cannot merge lanes into it's own lane"));
                }
                let mut other_ls = find_lane(&l_states, merge.lane)?
                    .ok_or_else(|| {
                        actor_error!(USR_ILLEGAL_ARGUMENT;
                        "voucher specifies invalid merge lane {}", merge.lane)
                    })?
                    .clone();

                if other_ls.nonce >= merge.nonce {
                    return Err(actor_error!(USR_ILLEGAL_ARGUMENT;
                            "merged lane in voucher has outdated nonce, cannot redeem"));
                }

                redeemed_from_others += &other_ls.redeemed;
                other_ls.nonce = merge.nonce;
                l_states.set(merge.lane, other_ls).map_err(|e| {
                    e.downcast_default(
                        ExitCode::USR_ILLEGAL_STATE,
                        format!("failed to store lane {}", merge.lane),
                    )
                })?;
            }

            // 2. To prevent double counting, remove already redeemed amounts (from
            // voucher or other lanes) from the voucher amount
            lane_state.nonce = sv.nonce;
            let balance_delta = &sv.amount - (redeemed_from_others + &lane_state.redeemed);

            // 3. set new redeemed value for merged-into lane
            lane_state.redeemed = sv.amount;

            // 4. check operation validity
            let new_send_balance = balance_delta + &st.to_send;

            if new_send_balance < TokenAmount::from(0) {
                return Err(actor_error!(USR_ILLEGAL_ARGUMENT;
                    "voucher would leave channel balance negative"));
            }

            if new_send_balance > rt.current_balance() {
                return Err(actor_error!(USR_ILLEGAL_ARGUMENT;
                    "not enough funds in channel to cover voucher"));
            }

            // 5. add new redemption ToSend
            st.to_send = new_send_balance;

            // update channel settlingAt and MinSettleHeight if delayed by voucher
            if sv.min_settle_height != 0 {
                if st.settling_at != 0 && st.settling_at < sv.min_settle_height {
                    st.settling_at = sv.min_settle_height;
                }
                if st.min_settle_height < sv.min_settle_height {
                    st.min_settle_height = sv.min_settle_height;
                }
            }

            l_states.set(lane_id, lane_state).map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    format!("failed to store lane {}", lane_id),
                )
            })?;

            st.lane_states = l_states.flush().map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to save lanes")
            })?;
            Ok(())
        })
    }

    pub fn settle<BS, RT>(rt: &mut RT) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        rt.transaction(|st: &mut State, rt| {
            rt.validate_immediate_caller_is([st.from, st.to].iter())?;

            if st.settling_at != 0 {
                return Err(actor_error!(USR_ILLEGAL_STATE; "channel already settling"));
            }

            st.settling_at = rt.curr_epoch() + SETTLE_DELAY;
            if st.settling_at < st.min_settle_height {
                st.settling_at = st.min_settle_height;
            }

            Ok(())
        })
    }

    pub fn collect<BS, RT>(rt: &mut RT) -> Result<(), ActorError>
    where
        BS: Blockstore,
        RT: Runtime<BS>,
    {
        let st: State = rt.state()?;
        rt.validate_immediate_caller_is(&[st.from, st.to])?;

        if st.settling_at == 0 || rt.curr_epoch() < st.settling_at {
            return Err(actor_error!(USR_FORBIDDEN; "payment channel not settling or settled"));
        }

        // send ToSend to `to`
        rt.send(st.to, METHOD_SEND, RawBytes::default(), st.to_send)
            .map_err(|e| e.wrap("Failed to send funds to `to` address"))?;

        // the remaining balance will be returned to "From" upon deletion.
        rt.delete_actor(&st.from)?;

        Ok(())
    }
}

#[inline]
fn find_lane<'a, BS>(
    ls: &'a Array<LaneState, BS>,
    id: u64,
) -> Result<Option<&'a LaneState>, ActorError>
where
    BS: Blockstore,
{
    if id > MAX_LANE {
        return Err(actor_error!(USR_ILLEGAL_ARGUMENT; "maximum lane ID is 2^63-1"));
    }

    ls.get(id).map_err(|e| {
        e.downcast_default(
            ExitCode::USR_ILLEGAL_STATE,
            format!("failed to load lane {}", id),
        )
    })
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
                Self::constructor(rt, rt.deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::UpdateChannelState) => {
                Self::update_channel_state(rt, rt.deserialize_params(params)?)?;
                Ok(RawBytes::default())
            }
            Some(Method::Settle) => {
                Self::settle(rt)?;
                Ok(RawBytes::default())
            }
            Some(Method::Collect) => {
                Self::collect(rt)?;
                Ok(RawBytes::default())
            }
            _ => Err(actor_error!(SYS_INVALID_METHOD; "Invalid method")),
        }
    }
}
