// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    ActorDowncast, ActorError, BURNT_FUNDS_ACTOR_ADDR, EXPECTED_LEADERS_PER_EPOCH,
    STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, actor_dispatch, actor_error, extract_send_result,
};

use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::{METHOD_CONSTRUCTOR, METHOD_SEND};
use log::{error, warn};
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
        let call = QueuedCall::Weights { op, updates: params.updates };
        let (apply_result, queued) = rt.transaction(|st: &mut State, rt| {
            let mut ledger = Ledger::load(rt, st)?;
            let apply_result = ledger.apply_due(rt.curr_epoch());
            let queued = ledger
                .admit(call, rt.curr_epoch(), st.swa_timelock_epochs)
                .map_err(|e| illegal_argument(e, "failed to queue weight records"))?
                .clone();
            ledger.store(rt, st)?;
            Ok((apply_result, queued))
        })?;
        complete_mutation(rt, &apply_result, Some(&queued), None)
    }

    /// Queues a new stream for activation no earlier than the SWA timelock.
    fn register_stream(rt: &impl Runtime, params: RegisterStreamParams) -> Result<(), ActorError> {
        validate_swa(rt)?;
        let distribution = params
            .distribution
            .map(|distribution| -> Result<DistributionInit, ActorError> {
                let shares = resolve_shares(rt, distribution.shares)?;
                Ok(DistributionInit {
                    writer: resolve_required(rt, &distribution.writer, "distribution writer")?,
                    shares: streams::normalize_shares(shares)
                        .map_err(|e| illegal_argument(e, "invalid initial shares"))?,
                })
            })
            .transpose()?;
        let call = QueuedCall::Register {
            id: params.id,
            weight: params.weight,
            distribution,
            activation: params.activation_epoch,
        };

        let (apply_result, queued) = rt.transaction(|st: &mut State, rt| {
            let mut ledger = Ledger::load(rt, st)?;
            let apply_result = ledger.apply_due(rt.curr_epoch());
            let queued = ledger
                .admit(call, rt.curr_epoch(), st.swa_timelock_epochs)
                .map_err(|e| illegal_argument(e, "failed to queue stream registration"))?
                .clone();
            ledger.store(rt, st)?;
            Ok((apply_result, queued))
        })?;
        complete_mutation(rt, &apply_result, Some(&queued), None)
    }

    /// Queues stream removal, preserving unpaid allocations when it applies.
    fn remove_stream(rt: &impl Runtime, params: RemoveStreamParams) -> Result<(), ActorError> {
        validate_swa(rt)?;
        let call = QueuedCall::Remove { id: params.id };
        let (apply_result, queued) = rt.transaction(|st: &mut State, rt| {
            let mut ledger = Ledger::load(rt, st)?;
            let apply_result = ledger.apply_due(rt.curr_epoch());
            let queued = ledger
                .admit(call, rt.curr_epoch(), st.swa_timelock_epochs)
                .map_err(|e| illegal_argument(e, "failed to queue stream removal"))?
                .clone();
            ledger.store(rt, st)?;
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
        let call = QueuedCall::SetDistribution { id: params.id, writer };
        let (apply_result, queued) = rt.transaction(|st: &mut State, rt| {
            let mut ledger = Ledger::load(rt, st)?;
            let apply_result = ledger.apply_due(rt.curr_epoch());
            let queued = ledger
                .admit(call, rt.curr_epoch(), st.swa_timelock_epochs)
                .map_err(|e| illegal_argument(e, "failed to queue distribution writer"))?
                .clone();
            ledger.store(rt, st)?;
            Ok((apply_result, queued))
        })?;
        complete_mutation(rt, &apply_result, Some(&queued), None)
    }

    /// Applies due writes, then removes the pending write in the named queue slot.
    fn cancel_pending(rt: &impl Runtime, params: CancelPendingParams) -> Result<(), ActorError> {
        validate_swa(rt)?;
        let slot = Slot::for_cancel(params.id, params.op)
            .map_err(|e| illegal_argument(e, "invalid cancellation target"))?;
        let (apply_result, cancelled) = rt.transaction(|st: &mut State, rt| {
            let mut ledger = Ledger::load(rt, st)?;
            let apply_result = ledger.apply_due(rt.curr_epoch());
            let cancelled = ledger.cancel(slot);
            ledger.store(rt, st)?;
            Ok((apply_result, cancelled))
        })?;
        complete_mutation(rt, &apply_result, None, cancelled.as_ref())
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
            let mut ledger = Ledger::load(rt, st)?;
            let mut apply_result = ledger.apply_due(rt.curr_epoch());
            let writer = ledger
                .streams()
                .explicit(params.id)
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
            let (streams, accrued) = ledger.mutate();
            let fold_dust = set_shares(streams, accrued, params.id, shares)
                .map_err(|e| illegal_argument(e, "failed to set stream shares"))?;
            apply_result.fold_dust += fold_dust;
            ledger.store(rt, st)?;
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
            let mut ledger = Ledger::load(rt, st)?;
            let apply_result = ledger.apply_due(rt.curr_epoch());
            let (streams, accrued) = ledger.mutate();
            let amounts = claim(streams, accrued, params.id, &lookup_wallets)
                .map_err(|e| illegal_argument(e, "failed to claim stream funds"))?;
            ledger.store(rt, st)?;
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
            let stream_bytes = rt
                .store()
                .get(&st.streams_root)
                .map_err(|error| {
                    actor_error!(
                        illegal_state,
                        "failed to load streams state {}: {}",
                        st.streams_root,
                        error
                    )
                })?
                .ok_or_else(|| {
                    actor_error!(
                        illegal_state,
                        "streams state root {} not found",
                        st.streams_root
                    )
                })?;
            let mut ledger = match Ledger::decode_for_award(&stream_bytes, &st.accrued) {
                Ok(ledger) => ledger,
                Err(error) => {
                    error!(
                        "invalid stream state at epoch {}: {}; paying gas reward only",
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
            let expected_block_reward: TokenAmount =
                (&st.this_epoch_reward * params.win_count).div_floor(EXPECTED_LEADERS_PER_EPOCH);

            // Due writes are projected into the ledger and stored only for a full award, so a
            // gas-only award doesn't commit a stream or accrual change.
            let apply_result = ledger.apply_due(rt.curr_epoch());
            let liabilities = match explicit_liability(ledger.streams(), ledger.accrued()) {
                Ok(liabilities) => liabilities,
                Err(error) => {
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
            };

            let mut block_reward = expected_block_reward;
            // Due folds leave dust out of the derived liability before its post-transaction
            // burn send, so it remains reserved until that send executes.
            let reserved = &params.gas_reward + &liabilities + &apply_result.fold_dust;
            if prior_balance <= reserved {
                warn!(
                    "reward balance {} does not exceed gas {}, explicit-stream liabilities {},\
                    and pending dust {}; paying gas reward only",
                    prior_balance, params.gas_reward, liabilities, apply_result.fold_dust
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

            let allocation =
                match allocate_reward(&ledger.streams().streams, rt.curr_epoch(), &block_reward) {
                    Ok(allocation) => allocation,
                    Err(error) => {
                        error!("failed to allocate reward at epoch {}: {}", rt.curr_epoch(), error);
                        return Ok((
                            params.gas_reward.clone(),
                            TokenAmount::zero(),
                            ApplyResult::default(),
                        ));
                    }
                };
            if !allocation.schedule_valid {
                warn!(
                    "invalid stream weights at epoch {}; paying gas reward only",
                    rt.curr_epoch()
                );
                return Ok((
                    params.gas_reward.clone(),
                    TokenAmount::zero(),
                    ApplyResult::default(),
                ));
            }
            accrue_explicit(ledger.accrued_mut(), &allocation.portions);
            let explicit = allocation
                .portions
                .iter()
                .fold(TokenAmount::zero(), |total, row| total + &row.amount);

            ledger.store(rt, st)?;
            st.total_minted_reward += &block_reward;
            st.total_burn_minted += &allocation.burn;
            st.total_explicit_minted += explicit;

            Ok((
                &params.gas_reward + allocation.miner,
                &apply_result.fold_dust + allocation.burn,
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
    let state: State = rt.state()?;
    rt.validate_immediate_caller_is(std::iter::once(&state.swa_actor))
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
    if result.fold_dust > TokenAmount::zero() {
        extract_send_result(rt.send_simple(
            &BURNT_FUNDS_ACTOR_ADDR,
            METHOD_SEND,
            None,
            result.fold_dust.clone(),
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
