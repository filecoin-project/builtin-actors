// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fil_actors_runtime::runtime::builtins::Type;
use fil_actors_runtime::runtime::{ActorCode, Runtime};
use fil_actors_runtime::{
    ActorDowncast, ActorError, BURNT_FUNDS_ACTOR_ADDR, EXPECTED_LEADERS_PER_EPOCH,
    STORAGE_POWER_ACTOR_ADDR, SYSTEM_ACTOR_ADDR, actor_dispatch, actor_error, extract_send_result,
};

use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::ipld_block::IpldBlock;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use fvm_shared::{ActorID, METHOD_CONSTRUCTOR, METHOD_SEND};
use log::{error, warn};
use num_derive::FromPrimitive;
use num_traits::Zero;

pub use self::logic::*;
pub use self::state::{
    DENOM, ExplicitDistribution, MAX_PAYABLE_ROWS_PER_STREAM, MAX_PENDING_WRITES, MAX_RECIPIENTS,
    MAX_STREAMS, MAX_TOMBSTONE_ROWS, PendingWrite, PendingWriteOp, RecipientAmount, RecipientShare,
    RecipientTable, State, Stream, StreamAccrual, StreamId, StreamsState, Tombstone, WeightRecord,
};
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
    ReplaceAddressExported = frc42_dispatch::method_hash!("ReplaceAddress"),
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
        let call = QueuedCall::Weights { op, updates: params.updates }.canonical();
        let (applied, queued) = run_mutation(rt, |ledger, epoch, timelock| {
            ledger
                .admit(call, epoch, timelock)
                .cloned()
                .map_err(|e| illegal_argument(e, "failed to queue weight records"))
        })?;
        settle_applied(rt, &applied)?;
        emit::write_queued(rt, &queued)
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
                    shares: streams::admit_shares(shares)
                        .map_err(|e| illegal_argument(e, "invalid initial shares"))?,
                })
            })
            .transpose()?;
        let call = QueuedCall::Register {
            id: params.id,
            weight: params.weight,
            distribution,
            activation: params.activation_epoch,
        }
        .canonical();

        let (applied, queued) = run_mutation(rt, |ledger, epoch, timelock| {
            ledger
                .admit(call, epoch, timelock)
                .cloned()
                .map_err(|e| illegal_argument(e, "failed to queue stream registration"))
        })?;
        settle_applied(rt, &applied)?;
        emit::write_queued(rt, &queued)
    }

    /// Queues stream removal, preserving unpaid allocations when it applies.
    fn remove_stream(rt: &impl Runtime, params: RemoveStreamParams) -> Result<(), ActorError> {
        validate_swa(rt)?;
        let call = QueuedCall::Remove { id: params.id }.canonical();
        let (applied, queued) = run_mutation(rt, |ledger, epoch, timelock| {
            ledger
                .admit(call, epoch, timelock)
                .cloned()
                .map_err(|e| illegal_argument(e, "failed to queue stream removal"))
        })?;
        settle_applied(rt, &applied)?;
        emit::write_queued(rt, &queued)
    }

    /// Queues replacement of an explicit stream's writer, closing its current period on apply.
    fn set_distribution(
        rt: &impl Runtime,
        params: SetDistributionParams,
    ) -> Result<(), ActorError> {
        validate_swa(rt)?;
        let writer = resolve_required(rt, &params.writer, "distribution writer")?;
        let call = QueuedCall::SetDistribution { id: params.id, writer }.canonical();
        let (applied, queued) = run_mutation(rt, |ledger, epoch, timelock| {
            ledger
                .admit(call, epoch, timelock)
                .cloned()
                .map_err(|e| illegal_argument(e, "failed to queue distribution writer"))
        })?;
        settle_applied(rt, &applied)?;
        emit::write_queued(rt, &queued)
    }

    /// Applies due writes, then removes the pending write in the named queue key.
    fn cancel_pending(rt: &impl Runtime, params: CancelPendingParams) -> Result<(), ActorError> {
        validate_swa(rt)?;
        let key = WriteKey::for_cancel(params.id, params.op)
            .map_err(|e| illegal_argument(e, "invalid cancellation target"))?;
        let (applied, cancelled) = run_mutation(rt, |ledger, _, _| Ok(ledger.cancel(key)))?;
        settle_applied(rt, &applied)?;
        if let Some(write) = cancelled {
            emit::write_cancelled(rt, &write)?;
        }
        Ok(())
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
        let (mut applied, (fold, installed)) = run_mutation(rt, |ledger, _, _| {
            // A due SetDistribution may have replaced the writer, so the check reads the ledger
            // the due writes left rather than the one this method loaded.
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
            // Recipient lookup stays behind the writer check, so an unauthorized call is
            // forbidden rather than not-found.
            let shares = resolve_shares(rt, params.shares)?;
            ledger
                .set_shares(params.id, shares)
                .map_err(|e| illegal_argument(e, "failed to set stream shares"))
        })?;
        // One burn send carries the immediate fold's dust with any the due writes left.
        applied.folds.push(fold);
        applied.installed.push(installed);
        settle_applied(rt, &applied)
    }

    /// Closes an explicit stream's period and moves one recipient's future share to a new
    /// address, for the stream writer only. Naming f099 drops the share to burn, like an f099 row
    /// in `SetShares`. The old wallet keeps its payable balance and the new wallet starts a fresh
    /// tally. This is `SetShares` on the stored map with one row changed, so the fold and every
    /// `SetShares` check apply. Either address may be given in any form that resolves.
    fn replace_address(rt: &impl Runtime, params: ReplaceAddressParams) -> Result<(), ActorError> {
        rt.validate_immediate_caller_accept_any()?;
        let caller = rt.message().caller();
        let (mut applied, (fold, old, new)) = run_mutation(rt, |ledger, _, _| {
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
            let old = resolve_required(rt, &params.old_address, "old recipient address")?;
            let new = resolve_recipient(rt, &params.new_address, "new recipient address")?;
            let fold = ledger
                .replace_address(params.id, old, new)
                .map_err(|e| illegal_argument(e, "failed to replace stream recipient address"))?;
            Ok((fold, old, new))
        })?;
        applied.folds.push(fold);
        settle_applied(rt, &applied)?;
        emit::address_replaced(rt, params.id, &old, &new)
    }

    /// Pays the named wallets' live and carried entitlements for one explicit stream.
    ///
    /// Anyone may call this method; amounts and payout events preserve request order.
    ///
    /// A claim always names a stream ID, and a stream that has been removed keeps answering under
    /// that same ID, because removal files its unpaid rows as a tombstone there. The tombstone
    /// deletes itself when its last row is claimed, and the ID returns zeros from then on. A
    /// wallet owed by several streams claims one stream at a time, live or tombstoned alike.
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
        // Stored recipients are ID addresses, so an unresolvable input takes a positional zero.
        let wallets: Vec<Option<Address>> = params
            .wallets
            .iter()
            .map(|wallet| rt.resolve_address(wallet).map(Address::new_id))
            .collect();

        let (applied, amounts) = run_mutation(rt, |ledger, _, _| {
            ledger
                .claim(params.id, &wallets)
                .map_err(|e| illegal_argument(e, "failed to claim stream funds"))
        })?;
        settle_applied(rt, &applied)?;
        for (wallet, amount) in wallets.iter().zip(&amounts) {
            if let Some(recipient) = wallet
                && amount > &TokenAmount::zero()
            {
                extract_send_result(rt.send_simple(recipient, METHOD_SEND, None, amount.clone()))?;
                emit::claim_payout(rt, params.id, recipient, amount)?;
            }
        }
        Ok(ClaimReturn { amounts })
    }

    /// Applies due stream writes and divides one block reward among all active streams.
    ///
    /// Explicit portions accrue for later claims. The implicit portion and gas reward go to the
    /// winning miner, while the exact residual is burnt. A failed miner or burn send is logged and
    /// the award stands, with the state committed. The system actor calls this implicitly once per
    /// block.
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

        let (miner_reward, mut burn, applied) = rt.transaction(|st: &mut State, rt| {
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
                    actor_error!(illegal_state, "streams state root {} not found", st.streams_root)
                })?;
            let ledger = match Ledger::decode_for_award(&stream_bytes, &st.accrued) {
                Ok(ledger) => ledger,
                Err(error) => {
                    error!(
                        "invalid stream state at epoch {}: {}; paying gas reward only",
                        rt.curr_epoch(),
                        error
                    );
                    return Ok(no_award(&params.gas_reward));
                }
            };
            let expected_block_reward: TokenAmount =
                (&st.this_epoch_reward * params.win_count).div_floor(EXPECTED_LEADERS_PER_EPOCH);

            // plan_award takes the ledger by value. On None it's dropped here, due writes and
            // all, so a gas-only award leaves the state exactly as it found it.
            let Some((ledger, award)) = plan_award(
                ledger,
                rt.curr_epoch(),
                &prior_balance,
                &params.gas_reward,
                &expected_block_reward,
            ) else {
                return Ok(no_award(&params.gas_reward));
            };

            if let Err(error) = ledger.validate_changes(st) {
                error!(
                    "award at epoch {} breaks the stream invariants: {}; paying gas reward only",
                    rt.curr_epoch(),
                    error
                );
                return Ok(no_award(&params.gas_reward));
            }

            let FullAward { block_reward, allocation, applied } = award;
            let miner_reward = &params.gas_reward + &allocation.miner;
            let burn = &applied.fold_dust() + &allocation.burn;
            // BR is capped to what remains after the gas reward, the streams' unclaimed balances
            // and the fold dust are set aside, and the split pays out exactly BR, so the outflow
            // should fit the balance and we shouldn't encounter this case.
            // Failure here would be an arithmetic/programmer bug but we handle it gracefully rather
            // than halt.
            if &miner_reward + &burn > prior_balance {
                error!(
                    "reward outflow {} exceeds balance {} at epoch {}; paying gas reward only",
                    &miner_reward + &burn,
                    prior_balance,
                    rt.curr_epoch()
                );
                return Ok(no_award(&params.gas_reward));
            }
            ledger.store(rt, st)?;
            st.total_minted_reward += &block_reward;
            st.total_burn_minted += &allocation.burn;
            st.total_explicit_minted +=
                allocation.portions.iter().map(|(_, amount)| amount).sum::<TokenAmount>();

            Ok((miner_reward, burn, applied))
        })?;

        // Implicit-message events are best-effort and would require FIP-0107 for chain visibility.
        if let Err(error) = emit_apply(rt, &applied) {
            warn!("failed to emit implicit award events: {error}");
        }
        let reward_params = ext::miner::ApplyRewardParams { reward: miner_reward.clone(), penalty };
        let miner_result = extract_send_result(rt.send_simple(
            &Address::new_id(miner_id),
            ext::miner::APPLY_REWARDS_METHOD,
            IpldBlock::serialize_cbor(&reward_params)?,
            miner_reward.clone(),
        ));

        // A miner that cannot take its reward has it burnt instead.
        if let Err(e) = miner_result {
            error!(
                "failed to send ApplyRewards call to the miner actor with funds {}, code: {:?}",
                miner_reward,
                e.exit_code()
            );
            burn += miner_reward;
        }
        // A failed burn leaves the award committed and its value with f02, where a later award
        // spends or burns it. An implicit call logs a send failure and returns Ok.
        if burn > TokenAmount::zero()
            && let Err(e) = extract_send_result(rt.send_simple(
                &BURNT_FUNDS_ACTOR_ADDR,
                METHOD_SEND,
                None,
                burn.clone(),
            ))
        {
            error!(
                "failed to send residual {} to the burnt funds actor, code: {:?}",
                burn,
                e.exit_code()
            );
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

fn resolve_existing(
    rt: &impl Runtime,
    address: &Address,
    label: &str,
) -> Result<(ActorID, Cid), ActorError> {
    let id = rt
        .resolve_address(address)
        .ok_or_else(|| actor_error!(not_found, "failed to resolve {} {}", label, address))?;
    let code = rt
        .get_actor_code_cid(&id)
        .ok_or_else(|| actor_error!(not_found, "{} {} does not exist", label, address))?;
    Ok((id, code))
}

fn resolve_required(
    rt: &impl Runtime,
    address: &Address,
    label: &str,
) -> Result<Address, ActorError> {
    resolve_existing(rt, address, label).map(|(id, _)| Address::new_id(id))
}

fn resolve_recipient(
    rt: &impl Runtime,
    address: &Address,
    label: &str,
) -> Result<Address, ActorError> {
    let (id, code) = resolve_existing(rt, address, label)?;
    let address = Address::new_id(id);
    // `Collect`` deletes payment channels, which can strand unpaid rewards, so they're disallowed
    // as recipients.
    if rt.resolve_builtin_actor_type(&code) == Some(Type::PaymentChannel) {
        return Err(actor_error!(illegal_argument, "{} {} is a payment channel", label, address));
    }
    Ok(address)
}

fn resolve_shares(
    rt: &impl Runtime,
    shares: Vec<RecipientShare>,
) -> Result<Vec<RecipientShare>, ActorError> {
    shares
        .into_iter()
        .map(|share| {
            Ok(RecipientShare {
                recipient: resolve_recipient(rt, &share.recipient, "share recipient")?,
                share: share.share,
            })
        })
        .collect()
}

fn illegal_argument(error: anyhow::Error, context: &'static str) -> ActorError {
    error.downcast_default(ExitCode::USR_ILLEGAL_ARGUMENT, context)
}

/// Called by every explicit method because they do the same thing: load the ledger, apply the
/// pending writes whose timelock has elapsed (the due writes), do the method's own work, and
/// store what it leaves.
///
/// The transaction is the atomicity boundary, so a rejection from `f` discards the whole thing,
/// due writes included. The epochs `f` receives are the current one and the SWA timelock.
fn run_mutation<T>(
    rt: &impl Runtime,
    f: impl FnOnce(&mut Ledger, ChainEpoch, ChainEpoch) -> Result<T, ActorError>,
) -> Result<(ApplyResult, T), ActorError> {
    rt.transaction(|st: &mut State, rt| {
        let mut ledger = Ledger::load(rt, st)?;
        let applied = ledger.apply_due(rt.curr_epoch());
        let value = f(&mut ledger, rt.curr_epoch(), st.swa_timelock_epochs)?;
        ledger.validate_changes(st).map_err(|e| {
            e.downcast_default(ExitCode::USR_ILLEGAL_STATE, "mutation breaks the stream invariants")
        })?;
        ledger.store(rt, st)?;
        Ok((applied, value))
    })
}

/// Emits an event for every write, then sends the fold dust it left to f099.
fn settle_applied(rt: &impl Runtime, applied: &ApplyResult) -> Result<(), ActorError> {
    emit_apply(rt, applied)?;
    let dust = applied.fold_dust();
    if dust > TokenAmount::zero() {
        extract_send_result(rt.send_simple(&BURNT_FUNDS_ACTOR_ADDR, METHOD_SEND, None, dust))?;
    }
    Ok(())
}

/// Announces the writes an application moved, then the periods it closed and the share maps it
/// installed. The award calls this on its own, because its burn includes the fold dust with the
/// block reward's residual.
fn emit_apply(rt: &impl Runtime, result: &ApplyResult) -> Result<(), ActorError> {
    for write in &result.applied {
        emit::write_applied(rt, write)?;
    }
    for write in &result.dropped {
        emit::write_dropped(rt, write)?;
    }
    for fold in &result.folds {
        emit::period_folded(rt, fold)?;
    }
    for installed in &result.installed {
        emit::shares_set(rt, installed)?;
    }
    Ok(())
}

/// FIP-0118 2.4.3's `no_award`: the miner is paid the gas reward alone and the state stands as it
/// was.
fn no_award(gas_reward: &TokenAmount) -> (TokenAmount, TokenAmount, ApplyResult) {
    (gas_reward.clone(), TokenAmount::zero(), ApplyResult::default())
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
        ReplaceAddressExported => replace_address,
        ClaimExported => claim,
    }
}

#[cfg(test)]
mod tests;
