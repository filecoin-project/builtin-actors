// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    ActorDowncast, ActorError, BURNT_FUNDS_ACTOR_ADDR, EXPECTED_LEADERS_PER_EPOCH,
    STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, actor_dispatch, actor_error, extract_send_result,
};

use fvm_ipld_encoding::{CborStore, ipld_block::IpldBlock};
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::{METHOD_CONSTRUCTOR, METHOD_SEND};
use log::{error, warn};
use multihash_codetable::Code;
use num_derive::FromPrimitive;
use num_traits::Zero;

pub use self::logic::*;
pub use self::state::State;
pub use self::streams::*;
pub use self::types::*;

#[cfg(feature = "fil-actor")]
fil_actors_runtime::wasm_trampoline!(Actor);

mod emit;
pub(crate) mod expneg;
mod logic;
mod state;
mod streams;
pub mod testing;
mod types;

// only exported for tests
#[doc(hidden)]
pub mod ext;

// * Updated to specs-actors commit: 999e57a151cc7ada020ca2844b651499ab8c0dec (v3.0.1)

/// PenaltyMultiplier is the factor miner penalties are scaled up by
pub const PENALTY_MULTIPLIER: u64 = 3;

lazy_static::lazy_static! {
    /// Temporary SWA identity used while the contract deployment address is unresolved.
    pub static ref MOCK_SWA_ACTOR_ADDR: Address = Address::new_id(1001);
}

/// Reward actor methods available
#[derive(FromPrimitive)]
#[repr(u64)]
pub enum Method {
    Constructor = METHOD_CONSTRUCTOR,
    AwardBlockReward = 2,
    ThisEpochReward = 3,
    UpdateNetworkKPI = 4,
    SetWeightRecordsExported = frc42_dispatch::method_hash!("SetWeightRecords"),
    StepWeightRecordsExported = frc42_dispatch::method_hash!("StepWeightRecords"),
    RegisterStreamExported = frc42_dispatch::method_hash!("RegisterStream"),
    RemoveStreamExported = frc42_dispatch::method_hash!("RemoveStream"),
    SetDistributionExported = frc42_dispatch::method_hash!("SetDistribution"),
    CancelPendingExported = frc42_dispatch::method_hash!("CancelPending"),
    SetSharesExported = frc42_dispatch::method_hash!("SetShares"),
    ClaimExported = frc42_dispatch::method_hash!("Claim"),
}

/// Reward Actor
pub struct Actor;

impl Actor {
    /// Constructor for Reward actor
    fn constructor(rt: &impl Runtime, params: ConstructorParams) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;

        if let Some(power) = params.power.map(|v| v.0) {
            let state = State::new(rt.store(), power).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to create reward state")
            })?;
            rt.create(&state)?;
            Ok(())
        } else {
            Err(actor_error!(illegal_argument, "argument should not be nil"))
        }
    }

    /// Queues a cancellable SWA update to stream weights.
    fn set_weight_records(
        rt: &impl Runtime,
        params: SetWeightRecordsParams,
    ) -> Result<(), ActorError> {
        Self::queue_weight_records(rt, params, PendingWriteOp::SetWeightRecords)
    }

    /// Queues an uncancellable gate-originated update to stream weights.
    fn step_weight_records(
        rt: &impl Runtime,
        params: StepWeightRecordsParams,
    ) -> Result<(), ActorError> {
        Self::queue_weight_records(rt, params, PendingWriteOp::StepWeightRecords)
    }

    fn queue_weight_records(
        rt: &impl Runtime,
        params: SetWeightRecordsParams,
        op: PendingWriteOp,
    ) -> Result<(), ActorError> {
        validate_swa(rt)?;
        let (apply_result, queued) = rt.transaction(|st: &mut State, rt| {
            let mut streams = load_streams(rt, st)?;
            let apply_result = apply_due(rt, st, &mut streams)?;
            queue_weight_records(
                &mut streams,
                rt.curr_epoch(),
                st.swa_timelock_epochs,
                op,
                &params.updates,
            )
            .map_err(|e| illegal_argument(e, "failed to queue weight records"))?;
            let queued = pending_write(&streams, None, op)?;
            store_streams(rt, st, &streams)?;
            Ok((apply_result, queued))
        })?;
        complete_mutation(rt, &apply_result, Some(&queued), None)
    }

    /// Queues a new stream for activation no earlier than the SWA timelock.
    fn register_stream(rt: &impl Runtime, params: RegisterStreamParams) -> Result<(), ActorError> {
        validate_swa(rt)?;
        let distribution = params
            .distribution
            .map(|distribution| -> Result<ExplicitDistribution, ActorError> {
                Ok(ExplicitDistribution {
                    writer: resolve_required(rt, &distribution.writer, "distribution writer")?,
                    shares: resolve_shares(rt, distribution.shares)?,
                    payable: Vec::new(),
                    claimed_period: Vec::new(),
                })
            })
            .transpose()?;
        let stream = Stream { id: params.id, weight: params.weight, distribution };

        let (apply_result, queued) = rt.transaction(|st: &mut State, rt| {
            let mut streams = load_streams(rt, st)?;
            let apply_result = apply_due(rt, st, &mut streams)?;
            queue_register_stream(
                &mut streams,
                rt.curr_epoch(),
                st.swa_timelock_epochs,
                stream,
                params.activation_epoch,
            )
            .map_err(|e| illegal_argument(e, "failed to queue stream registration"))?;
            let queued = pending_write(&streams, Some(params.id), PendingWriteOp::RegisterStream)?;
            store_streams(rt, st, &streams)?;
            Ok((apply_result, queued))
        })?;
        complete_mutation(rt, &apply_result, Some(&queued), None)
    }

    /// Queues stream removal, preserving unpaid allocations when it applies.
    fn remove_stream(rt: &impl Runtime, params: RemoveStreamParams) -> Result<(), ActorError> {
        validate_swa(rt)?;
        let (apply_result, queued) = rt.transaction(|st: &mut State, rt| {
            let mut streams = load_streams(rt, st)?;
            let apply_result = apply_due(rt, st, &mut streams)?;
            queue_remove_stream(&mut streams, rt.curr_epoch(), st.swa_timelock_epochs, params.id)
                .map_err(|e| illegal_argument(e, "failed to queue stream removal"))?;
            let queued = pending_write(&streams, Some(params.id), PendingWriteOp::RemoveStream)?;
            store_streams(rt, st, &streams)?;
            Ok((apply_result, queued))
        })?;
        complete_mutation(rt, &apply_result, Some(&queued), None)
    }

    /// Queues replacement of an explicit stream's writer, closing its current period on apply.
    fn set_distribution(
        rt: &impl Runtime,
        params: SetDistributionParams,
    ) -> Result<(), ActorError> {
        validate_swa(rt)?;
        let writer = resolve_required(rt, &params.writer, "distribution writer")?;
        let (apply_result, queued) = rt.transaction(|st: &mut State, rt| {
            let mut streams = load_streams(rt, st)?;
            let apply_result = apply_due(rt, st, &mut streams)?;
            queue_set_distribution(
                &mut streams,
                rt.curr_epoch(),
                st.swa_timelock_epochs,
                params.id,
                writer,
            )
            .map_err(|e| illegal_argument(e, "failed to queue distribution writer"))?;
            let queued = pending_write(&streams, Some(params.id), PendingWriteOp::SetDistribution)?;
            store_streams(rt, st, &streams)?;
            Ok((apply_result, queued))
        })?;
        complete_mutation(rt, &apply_result, Some(&queued), None)
    }

    /// Applies due writes, then removes the pending write in the named queue slot.
    fn cancel_pending(rt: &impl Runtime, params: CancelPendingParams) -> Result<(), ActorError> {
        validate_swa(rt)?;
        validate_cancel_target(params.id, params.op)
            .map_err(|e| illegal_argument(e, "invalid cancellation target"))?;
        let result = rt.transaction(|st: &mut State, rt| {
            let mut streams = load_streams(rt, st)?;
            validate_mutation_state(&streams, &st.accrued).map_err(|error| {
                error.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    "invalid stream state before cancellation",
                )
            })?;
            let result = apply_due_writes_and_cancel(
                &mut streams,
                &mut st.accrued,
                rt.curr_epoch(),
                params.id,
                params.op,
            )
            .map_err(|e| {
                e.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    "failed to apply due writes before cancellation",
                )
            })?;
            store_streams(rt, st, &streams)?;
            Ok(result)
        })?;
        complete_mutation(rt, &result.apply_result, None, result.removed.as_ref())
    }

    /// Closes an explicit stream's current period and installs its next recipient share map.
    fn set_shares(rt: &impl Runtime, params: SetSharesParams) -> Result<(), ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        if params.shares.len() > MAX_RECIPIENTS {
            return Err(actor_error!(
                illegal_argument,
                "recipient count {} exceeds maximum {}",
                params.shares.len(),
                MAX_RECIPIENTS
            ));
        }
        let caller = rt.message().caller();
        let apply_result = rt.transaction(|st: &mut State, rt| {
            let mut streams = load_streams(rt, st)?;
            let mut apply_result = apply_due(rt, st, &mut streams)?;
            let writer = streams
                .streams
                .iter()
                .find(|stream| stream.id == params.id)
                .and_then(|stream| stream.distribution.as_ref())
                .map(|distribution| distribution.writer)
                .ok_or_else(|| {
                    actor_error!(illegal_argument, "stream {} is not explicit", params.id)
                })?;
            if caller != writer {
                return Err(actor_error!(
                    forbidden,
                    "caller {} is not stream {} writer {}",
                    caller,
                    params.id,
                    writer
                ));
            }
            let shares = resolve_shares(rt, params.shares)?;
            let burn = set_shares(&mut streams, &mut st.accrued, params.id, shares)
                .map_err(|e| illegal_argument(e, "failed to set stream shares"))?;
            apply_result.burn += burn;
            store_streams(rt, st, &streams)?;
            Ok(apply_result)
        })?;
        complete_mutation(rt, &apply_result, None, None)
    }

    /// Pays the named wallets' live and carried entitlements for one explicit stream.
    ///
    /// Anyone may call this method; amounts and payout events preserve request order.
    fn claim(rt: &impl Runtime, params: ClaimParams) -> Result<ClaimReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        if params.wallets.len() > MAX_RECIPIENTS {
            return Err(actor_error!(
                illegal_argument,
                "wallet count {} exceeds maximum {}",
                params.wallets.len(),
                MAX_RECIPIENTS
            ));
        }
        let resolved_wallets: Vec<Option<Address>> = params
            .wallets
            .iter()
            .map(|wallet| rt.resolve_address(wallet).map(Address::new_id))
            .collect();
        // Stored recipients are ID addresses, so an unresolvable input produces a positional zero.
        let lookup_wallets: Vec<Address> = params
            .wallets
            .iter()
            .zip(&resolved_wallets)
            .map(|(original, resolved)| resolved.unwrap_or(*original))
            .collect();

        let (apply_result, amounts) = rt.transaction(|st: &mut State, rt| {
            let mut streams = load_streams(rt, st)?;
            let apply_result = apply_due(rt, st, &mut streams)?;
            let amounts = claim(&mut streams, &st.accrued, params.id, &lookup_wallets)
                .map_err(|e| illegal_argument(e, "failed to claim stream funds"))?;
            store_streams(rt, st, &streams)?;
            Ok((apply_result, amounts))
        })?;
        complete_apply(rt, &apply_result)?;
        for ((wallet, amount), resolved) in
            params.wallets.iter().zip(&amounts).zip(&resolved_wallets)
        {
            if amount <= &TokenAmount::zero() {
                continue;
            }
            let recipient = resolved.as_ref().ok_or_else(|| {
                actor_error!(illegal_state, "positive claim for unresolvable wallet {}", wallet)
            })?;
            extract_send_result(rt.send_simple(recipient, METHOD_SEND, None, amount.clone()))?;
            emit::claim_payout(rt, params.id, recipient, amount)?;
        }
        Ok(ClaimReturn { amounts })
    }

    /// Applies due stream writes and divides one block reward among all active streams.
    ///
    /// Explicit portions accrue for later claims. The implicit portion and gas reward go to the
    /// winning miner, while the exact residual is burnt. The system actor calls this implicitly
    /// once per block.
    fn award_block_reward(
        rt: &impl Runtime,
        params: AwardBlockRewardParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&SYSTEM_ACTOR_ADDR))?;
        let prior_balance = rt.current_balance();
        if params.penalty.is_negative() {
            return Err(actor_error!(illegal_argument, "negative penalty {}", params.penalty));
        }
        if params.gas_reward.is_negative() {
            return Err(actor_error!(
                illegal_argument,
                "negative gas reward {}",
                params.gas_reward
            ));
        }
        // The system actor must pass the exact message tips FVM credited to f02 before this call.
        if prior_balance < params.gas_reward {
            return Err(actor_error!(
                illegal_state,
                "actor current balance {} insufficient to pay gas reward {}",
                prior_balance,
                params.gas_reward
            ));
        }
        if params.win_count <= 0 {
            return Err(actor_error!(illegal_argument, "invalid win count {}", params.win_count));
        }

        let miner_id = rt
            .resolve_address(&params.miner)
            .ok_or_else(|| actor_error!(not_found, "failed to resolve given owner address"))?;
        let penalty: TokenAmount = &params.penalty * PENALTY_MULTIPLIER;

        let (miner_reward, burn, apply_result) = rt.transaction(|st: &mut State, rt| {
            let streams = load_streams(rt, st)?;
            validate_award_state_structure(&streams).map_err(|error| {
                error.downcast_default(
                    ExitCode::USR_ILLEGAL_STATE,
                    "invalid non-accounting stream state",
                )
            })?;
            if let Err(error) = compute_service_liability(&streams, &st.accrued) {
                error!(
                    "invalid explicit-stream accounting at epoch {}: {}; paying gas reward only",
                    rt.curr_epoch(),
                    error
                );
                return Ok((
                    params.gas_reward.clone(),
                    TokenAmount::zero(),
                    ApplyResult::default(),
                ));
            }

            // Project due writes separately so a degraded award commits no stream or accrual change.
            let mut next_streams = streams;
            let mut next_accrued = st.accrued.clone();
            let transition_due = next_streams
                .pending_writes
                .first()
                .is_some_and(|write| write.effective_epoch <= rt.curr_epoch());
            let apply_result = if transition_due {
                apply_due_writes(&mut next_streams, &mut next_accrued, rt.curr_epoch()).map_err(
                    |e| {
                        e.downcast_default(
                            ExitCode::USR_ILLEGAL_STATE,
                            "failed to apply due writes",
                        )
                    },
                )?
            } else {
                ApplyResult::default()
            };
            let liabilities = match compute_service_liability(&next_streams, &next_accrued) {
                Ok(liabilities) => liabilities,
                Err(error) => {
                    error!(
                        "due writes produced invalid explicit-stream accounting at epoch {}: {};\
                        paying gas reward only",
                        rt.curr_epoch(),
                        error
                    );
                    return Ok((
                        params.gas_reward.clone(),
                        TokenAmount::zero(),
                        ApplyResult::default(),
                    ));
                }
            };

            let mut block_reward: TokenAmount =
                (&st.this_epoch_reward * params.win_count).div_floor(EXPECTED_LEADERS_PER_EPOCH);
            // Due folds leave dust out of the derived liability before its post-transaction
            // burn send, so it remains reserved until that send executes.
            let reserved = &params.gas_reward + &liabilities + &apply_result.burn;
            if prior_balance < reserved {
                warn!(
                    "reward balance {} does not cover gas {}, explicit-stream liabilities {},\
                    and pending dust {}; paying gas reward only",
                    prior_balance, params.gas_reward, liabilities, apply_result.burn
                );
                return Ok((
                    params.gas_reward.clone(),
                    TokenAmount::zero(),
                    ApplyResult::default(),
                ));
            }
            let available_reward = &prior_balance - reserved;
            if block_reward > available_reward {
                warn!(
                    "reward actor spendable balance {} below block reward expected {},\
                    paying out spendable balance",
                    available_reward, block_reward
                );
                block_reward = available_reward;
            }

            // Structural and liability preflight guarantee a non-negative BR and one accrual row
            // for each unique explicit stream.
            let allocation = allocate_reward(&next_streams.streams, rt.curr_epoch(), &block_reward)
                .map_err(|e| {
                    e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to allocate reward")
                })?;
            if !allocation.schedule_valid {
                warn!(
                    "invalid stream weights at epoch {}; skipping explicit allocations",
                    rt.curr_epoch()
                );
            }
            accrue_service(&mut next_accrued, &allocation.service).map_err(|e| {
                e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to accrue service reward")
            })?;
            let service = allocation
                .service
                .iter()
                .fold(TokenAmount::zero(), |total, row| total + &row.amount);

            st.accrued = next_accrued;
            st.total_minted_reward += &block_reward;
            st.total_burn_minted += &allocation.burn;
            st.total_explicit_minted += service;
            if !(apply_result.applied.is_empty() && apply_result.dropped.is_empty()) {
                store_streams(rt, st, &next_streams)?;
            }

            Ok((
                &params.gas_reward + allocation.miner,
                &apply_result.burn + allocation.burn,
                apply_result,
            ))
        })?;

        // Reserved liabilities and dust are excluded before BR is capped; allocation conserves BR.
        let outgoing = &miner_reward + &burn;
        if outgoing > prior_balance {
            return Err(actor_error!(
                illegal_state,
                "reward outflow {} exceeds balance {}",
                outgoing,
                prior_balance
            ));
        }

        // Implicit-message events are best-effort and require FIP-0107 for chain visibility.
        if let Err(error) = emit_apply(rt, &apply_result) {
            warn!("failed to emit implicit award events: {error}");
        }
        let reward_params = ext::miner::ApplyRewardParams { reward: miner_reward.clone(), penalty };
        let miner_result = extract_send_result(rt.send_simple(
            &Address::new_id(miner_id),
            ext::miner::APPLY_REWARDS_METHOD,
            IpldBlock::serialize_cbor(&reward_params)?,
            miner_reward.clone(),
        ));

        match miner_result {
            Ok(_) => {
                if burn > TokenAmount::zero() {
                    extract_send_result(rt.send_simple(
                        &BURNT_FUNDS_ACTOR_ADDR,
                        METHOD_SEND,
                        None,
                        burn,
                    ))?;
                }
            }
            Err(e) => {
                error!(
                    "failed to send ApplyRewards call to the miner actor with funds {}, code: {:?}",
                    miner_reward,
                    e.exit_code()
                );
                let fallback_burn = burn + miner_reward;
                if fallback_burn > TokenAmount::zero()
                    && let Err(e) = extract_send_result(rt.send_simple(
                        &BURNT_FUNDS_ACTOR_ADDR,
                        METHOD_SEND,
                        None,
                        fallback_burn,
                    ))
                {
                    error!(
                        "failed to send unsent reward to the burnt funds actor, code: {:?}",
                        e.exit_code()
                    );
                }
            }
        }

        Ok(())
    }

    /// The award value used for the current epoch, updated at the end of an epoch
    /// through cron tick.  In the case previous epochs were null blocks this
    /// is the reward value as calculated at the last non-null epoch.
    fn this_epoch_reward(rt: &impl Runtime) -> Result<ThisEpochRewardReturn, ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let st: State = rt.state()?;
        Ok(ThisEpochRewardReturn {
            this_epoch_baseline_power: st.this_epoch_baseline_power,
            this_epoch_reward_smoothed: st.this_epoch_reward_smoothed,
        })
    }

    /// Called at the end of each epoch by the power actor (in turn by its cron hook).
    /// This is only invoked for non-empty tipsets, but catches up any number of null
    /// epochs to compute the next epoch reward.
    fn update_network_kpi(
        rt: &impl Runtime,
        params: UpdateNetworkKPIParams,
    ) -> Result<(), ActorError> {
        rt.validate_immediate_caller_is(std::iter::once(&STORAGE_POWER_ACTOR_ADDR))?;
        let curr_realized_power = params
            .curr_realized_power
            .ok_or_else(|| actor_error!(illegal_argument, "argument cannot be None"))?
            .0;

        rt.transaction(|st: &mut State, rt| {
            let prev = st.epoch;
            // if there were null runs catch up the computation until
            // st.Epoch == rt.CurrEpoch()
            while st.epoch < rt.curr_epoch() {
                // Update to next epoch to process null rounds
                st.update_to_next_epoch(&curr_realized_power);
            }

            st.update_to_next_epoch_with_reward(&curr_realized_power);
            st.update_smoothed_estimates(st.epoch - prev);
            Ok(())
        })?;
        Ok(())
    }
}

fn validate_swa(rt: &impl Runtime) -> Result<(), ActorError> {
    rt.validate_immediate_caller_is(std::iter::once(&*MOCK_SWA_ACTOR_ADDR))
}

fn resolve_required(
    rt: &impl Runtime,
    address: &Address,
    label: &str,
) -> Result<Address, ActorError> {
    let id = rt
        .resolve_address(address)
        .ok_or_else(|| actor_error!(not_found, "failed to resolve {} {}", label, address))?;
    if rt.get_actor_code_cid(&id).is_none() {
        return Err(actor_error!(not_found, "{} {} does not exist", label, address));
    }
    Ok(Address::new_id(id))
}

fn resolve_shares(
    rt: &impl Runtime,
    shares: Vec<RecipientShare>,
) -> Result<Vec<RecipientShare>, ActorError> {
    shares
        .into_iter()
        .map(|share| {
            Ok(RecipientShare {
                recipient: resolve_required(rt, &share.recipient, "share recipient")?,
                share: share.share,
            })
        })
        .collect()
}

fn load_streams(rt: &impl Runtime, state: &State) -> Result<StreamsState, ActorError> {
    rt.store()
        .get_cbor(&state.streams_root)
        .map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to load streams state")
        })?
        .ok_or_else(|| {
            actor_error!(illegal_state, "streams state root {} not found", state.streams_root)
        })
}

fn store_streams(
    rt: &impl Runtime,
    state: &mut State,
    streams: &StreamsState,
) -> Result<(), ActorError> {
    state.streams_root = rt.store().put_cbor(streams, Code::Blake2b256).map_err(|e| {
        e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to store streams state")
    })?;
    Ok(())
}

fn apply_due(
    rt: &impl Runtime,
    state: &mut State,
    streams: &mut StreamsState,
) -> Result<ApplyResult, ActorError> {
    let result = apply_due_writes(streams, &mut state.accrued, rt.curr_epoch()).map_err(|e| {
        e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "failed to apply due writes")
    })?;
    Ok(result)
}

fn pending_write(
    streams: &StreamsState,
    id: Option<StreamId>,
    op: PendingWriteOp,
) -> Result<PendingWrite, ActorError> {
    // Every caller reaches this only after its queue helper inserted the exact slot.
    streams
        .pending_writes
        .iter()
        .find(|write| write.id == id && write.op == op)
        .cloned()
        .ok_or_else(|| actor_error!(illegal_state, "queued write ({id:?}, {op:?}) not found"))
}

fn illegal_argument(error: anyhow::Error, context: &'static str) -> ActorError {
    error.downcast_default(ExitCode::USR_ILLEGAL_ARGUMENT, context)
}

fn complete_mutation(
    rt: &impl Runtime,
    apply_result: &ApplyResult,
    queued: Option<&PendingWrite>,
    cancelled: Option<&PendingWrite>,
) -> Result<(), ActorError> {
    complete_apply(rt, apply_result)?;
    if let Some(write) = queued {
        emit::write_queued(rt, write)?;
    }
    if let Some(write) = cancelled {
        emit::write_cancelled(rt, write)?;
    }
    Ok(())
}

fn emit_apply(rt: &impl Runtime, result: &ApplyResult) -> Result<(), ActorError> {
    for write in &result.applied {
        emit::write_applied(rt, write)?;
    }
    for write in &result.dropped {
        emit::write_dropped(rt, write)?;
    }
    Ok(())
}

fn complete_apply(rt: &impl Runtime, result: &ApplyResult) -> Result<(), ActorError> {
    emit_apply(rt, result)?;
    if result.burn > TokenAmount::zero() {
        extract_send_result(rt.send_simple(
            &BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            None,
            result.burn.clone(),
        ))?;
    }
    Ok(())
}

impl ActorCode for Actor {
    type Methods = Method;

    fn name() -> &'static str {
        "Reward"
    }

    actor_dispatch! {
        Constructor => constructor,
        AwardBlockReward => award_block_reward,
        ThisEpochReward => this_epoch_reward,
        UpdateNetworkKPI => update_network_kpi,
        SetWeightRecordsExported => set_weight_records,
        StepWeightRecordsExported => step_weight_records,
        RegisterStreamExported => register_stream,
        RemoveStreamExported => remove_stream,
        SetDistributionExported => set_distribution,
        CancelPendingExported => cancel_pending,
        SetSharesExported => set_shares,
        ClaimExported => claim,
    }
}
