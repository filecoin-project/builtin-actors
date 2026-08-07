// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::collections::BTreeSet;

use anyhow::Result;
use fvm_ipld_encoding::RawBytes;
use fvm_ipld_encoding::repr::*;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use num_traits::{ToPrimitive, Zero};

pub type StreamId = u64;

pub const DENOM: u64 = 1_000_000_000_000_000_000;
pub const MAX_STREAMS: usize = 8;
pub const MAX_RECIPIENTS: usize = 64;
pub const MAX_TOMBSTONE_ROWS: usize = 256;
pub const MAX_PENDING_WRITES: usize = MAX_STREAMS * 3 + 2;

////////////////////////////////////////////////////////////////////////////////
// Consensus-serialized types
////////////////////////////////////////////////////////////////////////////////
//
// Every type in this block is persisted in actor state, embedded in a deferred-write
// payload, or part of an actor method tuple. Field order and enum discriminants are wire format.

/// Stable operation tag persisted in `PendingWrite` and exposed by `CancelPending`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize_repr, Deserialize_repr)]
#[repr(u8)]
pub enum PendingWriteOp {
    SetWeightRecords = 0,
    /// Gate-originated weight update; unlike other operations, it cannot be cancelled.
    StepWeightRecords = 1,
    RegisterStream = 2,
    RemoveStream = 3,
    SetDistribution = 4,
}

/// A clamped linear weight in `DENOM` fixed point.
///
/// Persisted in `Stream` and encoded in weight-management messages and deferred payloads.
/// Admitted records satisfy `floor <= v_start <= cap <= DENOM`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct WeightRecord {
    pub v_start: u64,
    pub slope: i64,
    pub t_start: ChainEpoch,
    pub floor: u64,
    pub cap: u64,
}

/// One stream update in `SetWeightRecords` and `StepWeightRecords` parameters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct WeightRecordUpdate {
    pub id: StreamId,
    pub weight: WeightRecord,
}

/// Complete weight-update batch captured in one deferred call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
struct WeightRecordsPayload {
    updates: Vec<WeightRecordUpdate>,
}

/// One recipient entry in a share-map message and in persisted distribution state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RecipientShare {
    pub recipient: Address,
    pub share: u64,
}

/// Persisted allocation state for an explicit service stream.
///
/// The accounting rows are actor-owned state, not caller-supplied share-map fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct ExplicitDistribution {
    /// Designated share writer, not a payee.
    pub writer: Address,
    /// Current recipient fractions for the open share period.
    pub shares: Vec<RecipientShare>,
    /// Unclaimed allocations carried from closed share periods.
    pub payable: Vec<RecipientAmount>,
    /// Amounts already claimed against the current period's gross accrual.
    pub claimed_period: Vec<RecipientAmount>,
}

/// Persisted recipient balance in a live distribution or tombstone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RecipientAmount {
    pub recipient: Address,
    pub amount: TokenAmount,
}

/// A live stream persisted in `StreamsState`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct Stream {
    pub id: StreamId,
    pub weight: WeightRecord,
    /// None is the implicit consensus distribution; Some is an explicit service distribution.
    pub distribution: Option<ExplicitDistribution>,
}

/// Persisted liabilities for a removed stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct Tombstone {
    pub id: StreamId,
    pub payable: Vec<RecipientAmount>,
}

/// A deferred SWA operation persisted in `StreamsState`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct PendingWrite {
    /// Per-stream target, or `None` for a schedule-wide call.
    pub id: Option<StreamId>,
    pub op: PendingWriteOp,
    /// Operation-specific CBOR tuple.
    pub payload: RawBytes,
    pub effective_epoch: ChainEpoch,
}

/// Caller-supplied subset of a new explicit distribution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct DistributionInit {
    pub writer: Address,
    pub shares: Vec<RecipientShare>,
}

/// Private tuple stored in `PendingWrite.payload` for `RegisterStream`.
///
/// This is not the public actor-method parameter tuple.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RegisterStreamPayload {
    pub weight: WeightRecord,
    pub distribution: Option<DistributionInit>,
}

/// Private tuple stored in `PendingWrite.payload` for `SetDistribution`.
///
/// This is not the public actor-method parameter tuple.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct SetDistributionPayload {
    pub writer: Address,
}

/// Stream state persisted as the block referenced by `State.streams_root`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct StreamsState {
    /// Ordered by stream ID.
    pub streams: Vec<Stream>,
    /// Ordered by stream ID.
    pub tombstones: Vec<Tombstone>,
    /// Ordered by effective epoch; equal epochs retain queue position.
    pub pending_writes: Vec<PendingWrite>,
}

/// Current-period gross accrual persisted inline in `State`.
///
/// It stays outside `StreamsState` because it changes on every award.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct StreamAccrual {
    pub id: StreamId,
    pub amount: TokenAmount,
}

////////////////////////////////////////////////////////////////////////////////
// In-memory stream-engine effects
////////////////////////////////////////////////////////////////////////////////
//
// These values cross only Rust call boundaries and deliberately have no CBOR derives.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewardAllocation {
    pub miner: TokenAmount,
    pub service: Vec<StreamAccrual>,
    pub burn: TokenAmount,
    /// False when weight state is invalid; explicit portions are then skipped.
    pub schedule_valid: bool,
}

/// One claimed amount for each requested wallet, preserving request order.
pub type ClaimResult = Vec<TokenAmount>;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplyResult {
    pub burn: TokenAmount,
    /// Successful writes for actor-layer events after a committed application. Admission-only
    /// projections discard them.
    pub applied: Vec<PendingWrite>,
    /// Removed writes for actor-layer events after a committed application. Admission-only
    /// projections discard them.
    pub dropped: Vec<PendingWrite>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectedStreams {
    pub streams: StreamsState,
    pub accruals: Vec<StreamAccrual>,
    pub apply_result: ApplyResult,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelResult {
    pub apply_result: ApplyResult,
    /// Cancelled write for actor-layer events after a committed removal.
    pub removed: Option<PendingWrite>,
}

/// Evaluates a weight at `epoch`, clamped to its inclusive floor and cap.
pub fn compute_weight(record: &WeightRecord, epoch: ChainEpoch) -> u64 {
    let delta = BigInt::from(epoch) - BigInt::from(record.t_start);
    let value = BigInt::from(record.v_start) + BigInt::from(record.slope) * delta;
    // Both clamp bounds are u64, so even malformed ordering cannot leave the u64 domain.
    value
        .min(BigInt::from(record.cap))
        .max(BigInt::from(record.floor))
        .to_u64()
        .expect("bounded weight fits u64")
}

pub fn validate_weight_record(record: &WeightRecord) -> Result<()> {
    if record.floor > record.cap {
        return Err(anyhow::anyhow!("weight floor exceeds cap"));
    }
    if record.v_start < record.floor {
        return Err(anyhow::anyhow!("weight v_start is below floor"));
    }
    if record.v_start > record.cap {
        return Err(anyhow::anyhow!("weight v_start exceeds cap"));
    }
    if record.cap > DENOM {
        return Err(anyhow::anyhow!("weight cap exceeds DENOM"));
    }
    Ok(())
}

fn validate_weight_updates(updates: &[WeightRecordUpdate]) -> Result<()> {
    if updates.is_empty() {
        return Err(anyhow::anyhow!("weight-record update is empty"));
    }
    for pair in updates.windows(2) {
        if pair[0].id == pair[1].id {
            return Err(anyhow::anyhow!("duplicate weight-record stream ID {}", pair[0].id));
        }
        if pair[0].id > pair[1].id {
            return Err(anyhow::anyhow!("weight-record updates are not ordered"));
        }
    }
    for update in updates {
        validate_weight_record(&update.weight)?;
    }
    Ok(())
}

/// Returns epochs at or after `start_epoch` that can change an admitted record's regime.
/// Adjacent epochs bracket integer crossings so validation cannot skip a one-epoch violation.
pub fn weight_breakpoints(record: &WeightRecord, start_epoch: ChainEpoch) -> Vec<ChainEpoch> {
    // Include the validation boundary and any later record anchor.
    let mut epochs = BTreeSet::from([start_epoch]);
    if record.t_start >= start_epoch {
        epochs.insert(record.t_start);
    }

    if record.slope != 0 {
        // A canonical anchor has one crossing in the slope's forward direction. The opposite
        // crossing can matter only when validation begins before the anchor.
        let start = BigInt::from(record.t_start);
        let value = BigInt::from(record.v_start);
        let slope = BigInt::from(record.slope);
        let mut insert_crossing = |bound: u64| {
            let quotient = (BigInt::from(bound) - &value) / &slope;
            for offset in [-1_i64, 0, 1] {
                let epoch = &start + &quotient + offset;
                if let Some(epoch) = epoch.to_i64()
                    && epoch >= start_epoch
                {
                    epochs.insert(epoch);
                }
            }
        };

        insert_crossing(if record.slope > 0 { record.cap } else { record.floor });
        if start_epoch < record.t_start {
            insert_crossing(if record.slope > 0 { record.floor } else { record.cap });
        }
    }

    // Sample beyond the last crossing and at the epoch domain's absolute endpoint.
    if let Some(last) = epochs.last().copied()
        && let Some(after) = last.checked_add(1)
    {
        epochs.insert(after);
    }
    epochs.insert(ChainEpoch::MAX);
    epochs.into_iter().collect()
}

/// Validates aggregate stream weight from `start_epoch` onward.
pub fn validate_weight_schedule(streams: &[Stream], start_epoch: ChainEpoch) -> Result<()> {
    validate_weight_schedule_through(streams, start_epoch, ChainEpoch::MAX)
}

/// Validates aggregate stream weight over the inclusive epoch interval.
fn validate_weight_schedule_through(
    streams: &[Stream],
    start_epoch: ChainEpoch,
    end_epoch: ChainEpoch,
) -> Result<()> {
    if end_epoch < start_epoch {
        return Err(anyhow::anyhow!("weight schedule interval is reversed"));
    }
    let mut epochs = BTreeSet::from([start_epoch, end_epoch]);
    for stream in streams {
        validate_weight_record(&stream.weight)?;
        epochs.extend(
            weight_breakpoints(&stream.weight, start_epoch)
                .into_iter()
                .take_while(|epoch| *epoch <= end_epoch),
        );
    }

    for epoch in epochs {
        let sum: u128 =
            streams.iter().map(|stream| u128::from(compute_weight(&stream.weight, epoch))).sum();
        if sum > u128::from(DENOM) {
            return Err(anyhow::anyhow!("stream weights exceed DENOM at epoch {epoch}: {sum}"));
        }
    }
    Ok(())
}

/// Splits one block reward at `epoch`; invalid weight state allocates no portion.
pub fn allocate_reward(
    streams: &[Stream],
    epoch: ChainEpoch,
    block_reward: &TokenAmount,
) -> Result<RewardAllocation> {
    if block_reward.is_negative() {
        return Err(anyhow::anyhow!("block reward is negative"));
    }

    let mut miner = TokenAmount::zero();
    let mut service = Vec::with_capacity(streams.len());
    let mut allocated = TokenAmount::zero();
    let denom = BigInt::from(DENOM);
    let mut weight_sum = 0_u128;
    let mut records_valid = true;

    for stream in streams {
        records_valid &= validate_weight_record(&stream.weight).is_ok();
        let weight = compute_weight(&stream.weight, epoch);
        weight_sum = weight_sum.saturating_add(u128::from(weight));
        let portion = TokenAmount::from_atto(block_reward.atto() * weight / &denom);
        allocated += &portion;
        if stream.distribution.is_some() {
            service.push(StreamAccrual { id: stream.id, amount: portion });
        } else {
            miner += portion;
        }
    }

    let schedule_valid =
        records_valid && weight_sum <= u128::from(DENOM) && allocated <= *block_reward;
    if !schedule_valid {
        return Ok(RewardAllocation {
            miner: TokenAmount::zero(),
            service: Vec::new(),
            burn: TokenAmount::zero(),
            schedule_valid,
        });
    }

    Ok(RewardAllocation { miner, service, burn: block_reward - allocated, schedule_valid })
}

pub fn validate_shares(shares: &[RecipientShare]) -> Result<()> {
    if shares.len() > MAX_RECIPIENTS {
        return Err(anyhow::anyhow!(
            "recipient count {} exceeds maximum {MAX_RECIPIENTS}",
            shares.len()
        ));
    }

    let mut recipients = BTreeSet::new();
    let mut total = 0_u128;
    for row in shares {
        validate_id_address(&row.recipient, "share recipient")?;
        if row.share == 0 {
            return Err(anyhow::anyhow!("share for recipient {} is zero", row.recipient));
        }
        if !(recipients.insert(row.recipient)) {
            return Err(anyhow::anyhow!("duplicate share recipient {}", row.recipient));
        }
        total += u128::from(row.share);
    }
    if total != u128::from(DENOM) {
        return Err(anyhow::anyhow!("shares sum to {total}, expected {DENOM}"));
    }
    Ok(())
}

/// Adds this award's explicit-stream portions to their matching inline accruals.
pub fn accrue_service(accruals: &mut [StreamAccrual], portions: &[StreamAccrual]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for portion in portions {
        if portion.amount.is_negative() {
            return Err(anyhow::anyhow!("service portion is negative"));
        }
        if !(seen.insert(portion.id)) {
            return Err(anyhow::anyhow!("duplicate service portion for stream {}", portion.id));
        }
        if !(accruals.iter().any(|row| row.id == portion.id)) {
            return Err(anyhow::anyhow!("missing accrual for stream {}", portion.id));
        }
    }
    // The preflight above proves every lookup in this mutation pass succeeds.
    for portion in portions {
        let row = accruals
            .iter_mut()
            .find(|row| row.id == portion.id)
            .expect("explicit-stream accrual presence validated");
        row.amount += &portion.amount;
    }
    Ok(())
}

/// Computes explicit-stream funds still held by f02.
pub fn compute_service_liability(
    streams: &StreamsState,
    accruals: &[StreamAccrual],
) -> Result<TokenAmount> {
    let mut total = TokenAmount::zero();
    let mut accruals = accruals.iter();

    for stream in &streams.streams {
        let Some(distribution) = &stream.distribution else {
            // Implicit streams pay the miner directly and carry no service liability.
            continue;
        };
        let accrual = accruals
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing accrual for stream {}", stream.id))?;
        if accrual.id != stream.id {
            return Err(anyhow::anyhow!(
                "explicit-stream accrual {} does not match explicit stream {}",
                accrual.id,
                stream.id
            ));
        }
        if accrual.amount.is_negative() {
            return Err(anyhow::anyhow!(
                "explicit-stream accrual for stream {} is negative",
                stream.id
            ));
        }
        validate_period_claims(distribution, &accrual.amount)?;

        let claimed = distribution
            .claimed_period
            .iter()
            .fold(TokenAmount::zero(), |sum, row| sum + &row.amount);
        total += &accrual.amount - claimed;
        total +=
            distribution.payable.iter().fold(TokenAmount::zero(), |sum, row| sum + &row.amount);
    }
    if let Some(accrual) = accruals.next() {
        return Err(anyhow::anyhow!(
            "explicit-stream accrual {} has no matching explicit stream",
            accrual.id
        ));
    }
    for tombstone in &streams.tombstones {
        validate_amount_rows(&tombstone.payable, "tombstone payable")?;
        total += tombstone.payable.iter().fold(TokenAmount::zero(), |sum, row| sum + &row.amount);
    }
    Ok(total)
}

/// Closes the current period, preserves unclaimed earnings, and installs new shares.
/// Returns indivisible rounding dust for burning.
pub fn set_shares(
    streams: &mut StreamsState,
    accruals: &mut [StreamAccrual],
    id: StreamId,
    shares: Vec<RecipientShare>,
) -> Result<TokenAmount> {
    if !streams.pending_writes.iter().any(|write| write.op == PendingWriteOp::RemoveStream) {
        return set_shares_inner(streams, accruals, id, shares);
    }

    let mut next_streams = streams.clone();
    let mut next_accruals = accruals.to_vec();
    let burn = set_shares_inner(&mut next_streams, &mut next_accruals, id, shares)?;
    validate_tombstone_capacity(&next_streams)?;
    *streams = next_streams;
    // The slice signature prevents either path from changing the accrual row count.
    for (current, next) in accruals.iter_mut().zip(next_accruals) {
        *current = next;
    }
    Ok(burn)
}

fn set_shares_inner(
    streams: &mut StreamsState,
    accruals: &mut [StreamAccrual],
    id: StreamId,
    mut shares: Vec<RecipientShare>,
) -> Result<TokenAmount> {
    validate_shares(&shares)?;
    shares.sort_by_key(|row| row.recipient);

    let stream = streams
        .streams
        .iter_mut()
        .find(|stream| stream.id == id)
        .ok_or_else(|| anyhow::anyhow!("stream {id} not found"))?;
    let distribution =
        stream.distribution.as_mut().ok_or_else(|| anyhow::anyhow!("stream {id} is implicit"))?;
    let accrual = accruals
        .iter_mut()
        .find(|row| row.id == id)
        .ok_or_else(|| anyhow::anyhow!("missing accrual for stream {id}"))?;

    let burn = settle_period(distribution, &accrual.amount)?;
    accrual.amount = TokenAmount::zero();
    distribution.shares = shares;
    Ok(burn)
}

/// Claims live and carried earnings from either a registered stream or its tombstone.
pub fn claim(
    streams: &mut StreamsState,
    accruals: &[StreamAccrual],
    id: StreamId,
    wallets: &[Address],
) -> Result<ClaimResult> {
    if let Some(stream_idx) = streams.streams.iter().position(|stream| stream.id == id) {
        let stream = &mut streams.streams[stream_idx];
        let distribution = stream
            .distribution
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("stream {id} is implicit"))?;
        let pool = &accruals
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| anyhow::anyhow!("missing accrual for stream {id}"))?
            .amount;
        claim_live(distribution, pool, wallets)
    } else if let Some(tombstone_idx) =
        streams.tombstones.iter().position(|tombstone| tombstone.id == id)
    {
        let result = claim_payable(&mut streams.tombstones[tombstone_idx].payable, wallets)?;
        if streams.tombstones[tombstone_idx].payable.is_empty() {
            streams.tombstones.remove(tombstone_idx);
        }
        Ok(result)
    } else {
        Ok(vec![TokenAmount::zero(); wallets.len()])
    }
}

/// Carries current-period earnings into payable balances and returns rounding dust.
fn settle_period(
    distribution: &mut ExplicitDistribution,
    pool: &TokenAmount,
) -> Result<TokenAmount> {
    if pool.is_negative() {
        return Err(anyhow::anyhow!("explicit-stream accrual is negative"));
    }
    validate_shares(&distribution.shares)?;
    validate_period_claims(distribution, pool)?;

    let denom = BigInt::from(DENOM);
    let mut allocated = TokenAmount::zero();
    for share in &distribution.shares {
        let earned = TokenAmount::from_atto(pool.atto() * share.share / &denom);
        allocated += &earned;
        let claimed = amount_for(&distribution.claimed_period, &share.recipient);
        add_amount(&mut distribution.payable, share.recipient, earned - claimed);
    }
    distribution.claimed_period.clear();
    Ok(pool - allocated)
}

fn claim_live(
    distribution: &mut ExplicitDistribution,
    pool: &TokenAmount,
    wallets: &[Address],
) -> Result<ClaimResult> {
    if pool.is_negative() {
        return Err(anyhow::anyhow!("explicit-stream accrual is negative"));
    }
    validate_shares(&distribution.shares)?;
    validate_period_claims(distribution, pool)?;
    let denom = BigInt::from(DENOM);
    let mut amounts = Vec::with_capacity(wallets.len());

    for wallet in wallets {
        let share = distribution
            .shares
            .iter()
            .find(|row| row.recipient == *wallet)
            .map_or(0, |row| row.share);
        let earned = TokenAmount::from_atto(pool.atto() * share / &denom);
        let claimed = amount_for(&distribution.claimed_period, wallet);
        // validate_period_claims established this relation for every stored recipient.
        debug_assert!(claimed <= earned);
        let live = earned - claimed;
        let payable = amount_for(&distribution.payable, wallet);
        let entitlement = &live + &payable;
        if entitlement.is_zero() {
            amounts.push(TokenAmount::zero());
            continue;
        }
        remove_amount(&mut distribution.payable, wallet);
        add_amount(&mut distribution.claimed_period, *wallet, live);

        amounts.push(entitlement);
    }

    Ok(amounts)
}

fn claim_payable(payable: &mut Vec<RecipientAmount>, wallets: &[Address]) -> Result<ClaimResult> {
    validate_amount_rows(payable, "tombstone payable")?;
    let mut amounts = Vec::with_capacity(wallets.len());

    for wallet in wallets {
        let entitlement = amount_for(payable, wallet);
        if entitlement.is_zero() {
            amounts.push(TokenAmount::zero());
            continue;
        }
        remove_amount(payable, wallet);

        amounts.push(entitlement);
    }
    Ok(amounts)
}

fn validate_period_claims(distribution: &ExplicitDistribution, pool: &TokenAmount) -> Result<()> {
    validate_amount_rows(&distribution.payable, "payable")?;
    validate_amount_rows(&distribution.claimed_period, "claimed-period")?;
    let denom = BigInt::from(DENOM);
    for claimed in &distribution.claimed_period {
        let share =
            distribution
                .shares
                .iter()
                .find(|share| share.recipient == claimed.recipient)
                .ok_or_else(|| anyhow::anyhow!("claimed-period recipient is absent from shares"))?;
        let earned = TokenAmount::from_atto(pool.atto() * share.share / &denom);
        if claimed.amount > earned {
            return Err(anyhow::anyhow!(
                "claimed amount exceeds earnings for recipient {}",
                claimed.recipient
            ));
        }
    }
    Ok(())
}

fn validate_amount_rows(rows: &[RecipientAmount], label: &str) -> Result<()> {
    if !(rows.windows(2).all(|rows| rows[0].recipient < rows[1].recipient)) {
        return Err(anyhow::anyhow!("{label} recipients are not ordered"));
    }
    for row in rows {
        validate_id_address(&row.recipient, label)?;
        if row.amount <= TokenAmount::zero() {
            return Err(anyhow::anyhow!("{label} amount is not positive"));
        }
    }
    Ok(())
}

fn amount_for(rows: &[RecipientAmount], recipient: &Address) -> TokenAmount {
    rows.binary_search_by(|row| row.recipient.cmp(recipient))
        .map_or_else(|_| TokenAmount::zero(), |idx| rows[idx].amount.clone())
}

fn add_amount(rows: &mut Vec<RecipientAmount>, recipient: Address, amount: TokenAmount) {
    if amount.is_zero() {
        return;
    }
    match rows.binary_search_by(|row| row.recipient.cmp(&recipient)) {
        Ok(idx) => rows[idx].amount += amount,
        Err(idx) => rows.insert(idx, RecipientAmount { recipient, amount }),
    }
}

fn remove_amount(rows: &mut Vec<RecipientAmount>, recipient: &Address) -> TokenAmount {
    rows.binary_search_by(|row| row.recipient.cmp(recipient))
        .map_or_else(|_| TokenAmount::zero(), |idx| rows.remove(idx).amount)
}

/// Queues a weight batch at the timelock boundary after validating all projected writes.
pub fn queue_weight_records(
    streams: &mut StreamsState,
    current_epoch: ChainEpoch,
    timelock_epochs: ChainEpoch,
    op: PendingWriteOp,
    updates: &[WeightRecordUpdate],
) -> Result<ChainEpoch> {
    if !(matches!(op, PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords)) {
        return Err(anyhow::anyhow!("invalid weight-record operation {op:?}"));
    }
    let effective_epoch = timelock_epoch(current_epoch, timelock_epochs)?;
    let mut updates = updates.to_vec();
    updates.sort_by_key(|update| update.id);
    validate_weight_updates(&updates)?;

    let mut proposed = streams.clone();
    ensure_slot_available(&proposed, None, op)?;
    proposed.pending_writes.push(PendingWrite {
        id: None,
        op,
        payload: RawBytes::serialize(&WeightRecordsPayload { updates })?,
        effective_epoch,
    });
    sort_pending(&mut proposed.pending_writes);
    validate_new_pending(streams, &proposed, current_epoch, (None, op))?;

    *streams = proposed;
    Ok(effective_epoch)
}

/// Queues registration at an activation epoch no earlier than the timelock boundary.
pub fn queue_register_stream(
    streams: &mut StreamsState,
    current_epoch: ChainEpoch,
    timelock_epochs: ChainEpoch,
    stream: Stream,
    activation_epoch: ChainEpoch,
) -> Result<ChainEpoch> {
    if stream.id == 0 {
        return Err(anyhow::anyhow!("stream ID 0 is reserved"));
    }
    validate_weight_record(&stream.weight)?;
    if let Some(distribution) = &stream.distribution
        && (!distribution.payable.is_empty() || !distribution.claimed_period.is_empty())
    {
        return Err(anyhow::anyhow!("new stream has pre-existing accounting state"));
    }
    let mut distribution = stream.distribution.map(|distribution| DistributionInit {
        writer: distribution.writer,
        shares: distribution.shares,
    });
    if let Some(distribution) = &mut distribution {
        distribution.shares.sort_by_key(|row| row.recipient);
    }
    validate_distribution_init(&distribution)?;
    ensure_stream_id_available(streams, stream.id)?;
    ensure_slot_available(streams, Some(stream.id), PendingWriteOp::RegisterStream)?;

    let earliest = timelock_epoch(current_epoch, timelock_epochs)?;
    if activation_epoch < earliest {
        return Err(anyhow::anyhow!(
            "activation epoch {activation_epoch} is before timelock floor {earliest}"
        ));
    }

    let mut proposed = streams.clone();
    proposed.pending_writes.push(PendingWrite {
        id: Some(stream.id),
        op: PendingWriteOp::RegisterStream,
        payload: RawBytes::serialize(&RegisterStreamPayload {
            weight: stream.weight,
            distribution,
        })?,
        effective_epoch: activation_epoch,
    });
    sort_pending(&mut proposed.pending_writes);
    validate_new_pending(
        streams,
        &proposed,
        current_epoch,
        (Some(stream.id), PendingWriteOp::RegisterStream),
    )?;

    *streams = proposed;
    Ok(activation_epoch)
}

/// Queues removal; explicit-stream liabilities are settled when the write applies.
pub fn queue_remove_stream(
    streams: &mut StreamsState,
    current_epoch: ChainEpoch,
    timelock_epochs: ChainEpoch,
    id: StreamId,
) -> Result<ChainEpoch> {
    let effective_epoch = timelock_epoch(current_epoch, timelock_epochs)?;
    ensure_slot_available(streams, Some(id), PendingWriteOp::RemoveStream)?;

    let mut proposed = streams.clone();
    proposed.pending_writes.push(PendingWrite {
        id: Some(id),
        op: PendingWriteOp::RemoveStream,
        payload: RawBytes::new(vec![0x80]),
        effective_epoch,
    });
    sort_pending(&mut proposed.pending_writes);
    validate_new_pending(
        streams,
        &proposed,
        current_epoch,
        (Some(id), PendingWriteOp::RemoveStream),
    )?;
    validate_tombstone_capacity(&proposed)?;

    *streams = proposed;
    Ok(effective_epoch)
}

/// Queues writer replacement; the outgoing period is settled when the write applies.
pub fn queue_set_distribution(
    streams: &mut StreamsState,
    current_epoch: ChainEpoch,
    timelock_epochs: ChainEpoch,
    id: StreamId,
    writer: Address,
) -> Result<ChainEpoch> {
    let effective_epoch = timelock_epoch(current_epoch, timelock_epochs)?;
    ensure_slot_available(streams, Some(id), PendingWriteOp::SetDistribution)?;

    let mut proposed = streams.clone();
    proposed.pending_writes.push(PendingWrite {
        id: Some(id),
        op: PendingWriteOp::SetDistribution,
        payload: RawBytes::serialize(&SetDistributionPayload { writer })?,
        effective_epoch,
    });
    sort_pending(&mut proposed.pending_writes);
    validate_new_pending(
        streams,
        &proposed,
        current_epoch,
        (Some(id), PendingWriteOp::SetDistribution),
    )?;

    *streams = proposed;
    Ok(effective_epoch)
}

fn cancel_pending(
    streams: &mut StreamsState,
    id: Option<StreamId>,
    op: PendingWriteOp,
) -> Result<Option<PendingWrite>> {
    validate_cancel_target(id, op)?;
    let slot = pending_slot(id, op);
    let removed = streams
        .pending_writes
        .iter()
        .position(|write| pending_slot(write.id, write.op) == slot)
        .map(|idx| streams.pending_writes.remove(idx));
    Ok(removed)
}

pub(crate) fn validate_cancel_target(id: Option<StreamId>, op: PendingWriteOp) -> Result<()> {
    if op == PendingWriteOp::StepWeightRecords {
        return Err(anyhow::anyhow!("StepWeightRecords cannot be cancelled"));
    }
    validate_pending_target(id, op)
}

/// Applies writes due through `epoch` before attempting cancellation.
pub fn apply_due_writes_and_cancel(
    streams: &mut StreamsState,
    accruals: &mut Vec<StreamAccrual>,
    epoch: ChainEpoch,
    id: Option<StreamId>,
    op: PendingWriteOp,
) -> Result<CancelResult> {
    validate_mutation_state(streams, accruals)?;
    let mut projected = project_due_writes(streams, accruals, epoch)?;
    let removed = cancel_pending(&mut projected.streams, id, op)?;
    *streams = projected.streams;
    *accruals = projected.accruals;
    Ok(CancelResult { apply_result: projected.apply_result, removed })
}

/// Applies every write due through `epoch`. From invalid weight state, only writes that restore
/// a valid schedule apply; the rest are dropped atomically.
pub fn apply_due_writes(
    streams: &mut StreamsState,
    accruals: &mut Vec<StreamAccrual>,
    epoch: ChainEpoch,
) -> Result<ApplyResult> {
    validate_mutation_state(streams, accruals)?;
    if streams.pending_writes.first().is_none_or(|write| write.effective_epoch > epoch) {
        return Ok(ApplyResult::default());
    }

    let mut next_streams = streams.clone();
    let mut next_accruals = accruals.clone();

    let due_count = next_streams
        .pending_writes
        .iter()
        .take_while(|write| write.effective_epoch <= epoch)
        .count();
    let due: Vec<_> = next_streams.pending_writes.drain(..due_count).collect();
    let mut burn = TokenAmount::zero();
    let mut applied = Vec::new();
    let mut dropped = Vec::new();

    for write in due {
        let mut projected = next_streams.clone();
        let stranded = apply_pending_transition(&mut projected, None, &write)
            .and_then(|_| validate_transition_state(&projected, epoch))
            .is_err();
        if stranded {
            dropped.push(write);
            continue;
        }

        let mut candidate_streams = next_streams.clone();
        let mut candidate_accruals = next_accruals.clone();
        let write_burn = apply_pending_transition(
            &mut candidate_streams,
            Some(&mut candidate_accruals),
            &write,
        )?;
        validate_transition_state(&candidate_streams, epoch)?;
        next_streams = candidate_streams;
        next_accruals = candidate_accruals;
        applied.push(write);
        burn += write_burn;
    }

    *streams = next_streams;
    *accruals = next_accruals;
    Ok(ApplyResult { burn, applied, dropped })
}

/// Projects due writes without mutating the supplied state.
pub fn project_due_writes(
    streams: &StreamsState,
    accruals: &[StreamAccrual],
    epoch: ChainEpoch,
) -> Result<ProjectedStreams> {
    let mut projected_streams = streams.clone();
    let mut projected_accruals = accruals.to_vec();
    let apply_result = apply_due_writes(&mut projected_streams, &mut projected_accruals, epoch)?;
    Ok(ProjectedStreams { streams: projected_streams, accruals: projected_accruals, apply_result })
}

fn timelock_epoch(current_epoch: ChainEpoch, timelock_epochs: ChainEpoch) -> Result<ChainEpoch> {
    if timelock_epochs < 0 {
        return Err(anyhow::anyhow!("timelock is negative"));
    }
    current_epoch
        .checked_add(timelock_epochs)
        .ok_or_else(|| anyhow::anyhow!("timelock epoch overflow"))
}

fn pending_slot(id: Option<StreamId>, op: PendingWriteOp) -> (Option<StreamId>, PendingWriteOp) {
    match op {
        PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords => (None, op),
        _ => (id, op),
    }
}

fn validate_pending_target(id: Option<StreamId>, op: PendingWriteOp) -> Result<()> {
    let schedule_wide =
        matches!(op, PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords);
    match (schedule_wide, id) {
        (true, Some(_)) => Err(anyhow::anyhow!("schedule-wide call has a stream ID")),
        (false, None) => Err(anyhow::anyhow!("per-stream call has no stream ID")),
        _ => Ok(()),
    }
}

fn ensure_slot_available(
    streams: &StreamsState,
    id: Option<StreamId>,
    op: PendingWriteOp,
) -> Result<()> {
    let slot = pending_slot(id, op);
    if streams.pending_writes.iter().any(|write| pending_slot(write.id, write.op) == slot) {
        return Err(anyhow::anyhow!("pending slot ({:?}, {:?}) is occupied", slot.0, slot.1));
    }
    Ok(())
}

fn ensure_stream_id_available(streams: &StreamsState, id: StreamId) -> Result<()> {
    if streams.streams.iter().any(|stream| stream.id == id) {
        return Err(anyhow::anyhow!("stream ID {id} is already registered"));
    }
    if streams.tombstones.iter().any(|tombstone| tombstone.id == id) {
        return Err(anyhow::anyhow!("stream ID {id} is tombstoned"));
    }
    if streams
        .pending_writes
        .iter()
        .any(|write| write.id == Some(id) && write.op == PendingWriteOp::RegisterStream)
    {
        return Err(anyhow::anyhow!("stream ID {id} has a pending registration"));
    }
    Ok(())
}

pub(crate) fn validate_mutation_state(
    streams: &StreamsState,
    accruals: &[StreamAccrual],
) -> Result<()> {
    validate_streams_state_structure_without_weights(streams, accruals)
}

/// Stable sorting preserves insertion order among calls effective at the same epoch.
fn sort_pending(writes: &mut [PendingWrite]) {
    writes.sort_by_key(|write| write.effective_epoch);
}

fn validate_new_pending(
    current: &StreamsState,
    proposed: &StreamsState,
    current_epoch: ChainEpoch,
    new_slot: (Option<StreamId>, PendingWriteOp),
) -> Result<()> {
    let accepted_before = match validate_projected_queue(current, current_epoch, None) {
        Ok(accepted) => accepted,
        Err(error) if new_slot.1 == PendingWriteOp::SetWeightRecords => {
            validate_projected_queue_recovering(current, current_epoch, None).map_err(|_| error)?
        }
        Err(error) => return Err(error),
    };
    let accepted_after = match validate_projected_queue(proposed, current_epoch, Some(new_slot)) {
        Ok(accepted) => accepted,
        Err(error) if new_slot.1 == PendingWriteOp::SetWeightRecords => {
            validate_projected_queue_recovering(proposed, current_epoch, Some(new_slot))
                .map_err(|_| error)?
        }
        Err(error) => return Err(error),
    };
    if !accepted_before.is_subset(&accepted_after) {
        return Err(anyhow::anyhow!("new call invalidates an existing pending call"));
    }
    Ok(())
}

/// Projects calls in execution order. Calls stranded by cancellation become future drops.
fn validate_projected_queue(
    streams: &StreamsState,
    current_epoch: ChainEpoch,
    required_slot: Option<(Option<StreamId>, PendingWriteOp)>,
) -> Result<BTreeSet<(Option<StreamId>, PendingWriteOp)>> {
    validate_projected_queue_inner(
        streams,
        current_epoch,
        required_slot,
        false,
        Some(current_epoch),
    )
}

/// Projects a repair from otherwise well-formed state with invalid weight records or envelope.
fn validate_projected_queue_recovering(
    streams: &StreamsState,
    current_epoch: ChainEpoch,
    required_slot: Option<(Option<StreamId>, PendingWriteOp)>,
) -> Result<BTreeSet<(Option<StreamId>, PendingWriteOp)>> {
    validate_projected_queue_inner(streams, current_epoch, required_slot, true, Some(current_epoch))
}

fn validate_projected_queue_inner(
    streams: &StreamsState,
    current_epoch: ChainEpoch,
    required_slot: Option<(Option<StreamId>, PendingWriteOp)>,
    allow_invalid_initial_schedule: bool,
    minimum_epoch: Option<ChainEpoch>,
) -> Result<BTreeSet<(Option<StreamId>, PendingWriteOp)>> {
    validate_pending_queue(&streams.pending_writes, minimum_epoch)?;
    if allow_invalid_initial_schedule {
        validate_stream_configuration_without_weights(&streams.streams)?;
    } else {
        validate_stream_configuration(&streams.streams)?;
        validate_weight_schedule(&streams.streams, current_epoch)?;
    }
    let mut projected = streams.clone();
    let mut accepted = BTreeSet::new();

    for write in &streams.pending_writes {
        let slot = pending_slot(write.id, write.op);
        // Admission starts at the entry's effective epoch; application starts at the current
        // epoch. Admission is intentionally stricter, and both choices are deterministic.
        let mut candidate = projected.clone();
        let result = apply_pending_transition(&mut candidate, None, write)
            .and_then(|_| validate_transition_state(&candidate, write.effective_epoch));
        match result {
            Ok(()) => {
                projected = candidate;
                accepted.insert(slot);
            }
            Err(error) if Some(slot) == required_slot => {
                return Err(anyhow::anyhow!(
                    "pending call ({:?}, {:?}) is invalid: {error}",
                    write.id,
                    write.op
                ));
            }
            Err(_) => {}
        }
    }
    Ok(accepted)
}

fn validate_transition_state(streams: &StreamsState, start_epoch: ChainEpoch) -> Result<()> {
    validate_stream_configuration(&streams.streams)?;
    validate_weight_schedule(&streams.streams, start_epoch)?;
    validate_tombstone_capacity(streams)
}

fn validate_stream_configuration(streams: &[Stream]) -> Result<()> {
    validate_stream_configuration_without_weights(streams)?;
    for stream in streams {
        validate_weight_record(&stream.weight)?;
    }
    Ok(())
}

fn validate_stream_configuration_without_weights(streams: &[Stream]) -> Result<()> {
    if streams.len() > MAX_STREAMS {
        return Err(anyhow::anyhow!("stream count exceeds maximum {MAX_STREAMS}"));
    }
    if !(streams.windows(2).all(|rows| rows[0].id < rows[1].id)) {
        return Err(anyhow::anyhow!("stream IDs are not ordered"));
    }
    if streams.iter().any(|stream| stream.id == 0) {
        return Err(anyhow::anyhow!("stream ID 0 is reserved"));
    }
    if streams.iter().filter(|stream| stream.distribution.is_none()).count() > 1 {
        return Err(anyhow::anyhow!("multiple implicit streams"));
    }
    for stream in streams {
        if let Some(distribution) = &stream.distribution {
            validate_id_address(&distribution.writer, "distribution writer")?;
            validate_shares(&distribution.shares)?;
            if !(distribution.shares.windows(2).all(|rows| rows[0].recipient < rows[1].recipient)) {
                return Err(anyhow::anyhow!("share recipients are not ordered"));
            }
            validate_amount_rows(&distribution.payable, "payable")?;
            validate_amount_rows(&distribution.claimed_period, "claimed-period")?;
        }
    }
    Ok(())
}

fn validate_pending_queue(
    writes: &[PendingWrite],
    minimum_epoch: Option<ChainEpoch>,
) -> Result<()> {
    if !(writes.windows(2).all(|rows| rows[0].effective_epoch <= rows[1].effective_epoch)) {
        return Err(anyhow::anyhow!("pending writes are not ordered"));
    }
    if writes.len() > MAX_PENDING_WRITES {
        return Err(anyhow::anyhow!(
            "pending write count {} exceeds maximum {MAX_PENDING_WRITES}",
            writes.len()
        ));
    }
    let mut slots = BTreeSet::new();
    for write in writes {
        let is_schedule = matches!(
            write.op,
            PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords
        );
        if is_schedule != write.id.is_none() {
            return Err(anyhow::anyhow!(
                "pending call ({:?}, {:?}) has a non-canonical stream ID",
                write.id,
                write.op
            ));
        }
        validate_pending_payload(write)?;
        let slot = pending_slot(write.id, write.op);
        if !slots.insert(slot) {
            return Err(anyhow::anyhow!("duplicate pending slot ({:?}, {:?})", slot.0, slot.1));
        }
        if let Some(epoch) = minimum_epoch
            && write.effective_epoch < epoch
        {
            return Err(anyhow::anyhow!(
                "pending write ({:?}, {:?}) is in the past",
                write.id,
                write.op
            ));
        }
    }
    Ok(())
}

fn validate_pending_payload(write: &PendingWrite) -> Result<()> {
    match write.op {
        PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords => {
            let payload: WeightRecordsPayload = write.payload.deserialize()?;
            validate_weight_updates(&payload.updates)?;
        }
        PendingWriteOp::RegisterStream => {
            let payload: RegisterStreamPayload = write.payload.deserialize()?;
            validate_weight_record(&payload.weight)?;
            validate_distribution_init(&payload.distribution)?;
        }
        PendingWriteOp::RemoveStream => {
            if write.payload.bytes() != [0x80] {
                return Err(anyhow::anyhow!("RemoveStream payload is not an empty tuple"));
            }
        }
        PendingWriteOp::SetDistribution => {
            let payload: SetDistributionPayload = write.payload.deserialize()?;
            validate_id_address(&payload.writer, "distribution writer")?;
        }
    }
    Ok(())
}

/// Validates persisted stream state and its queued schedule at `current_epoch`.
pub fn validate_streams_state(
    streams: &StreamsState,
    accruals: &[StreamAccrual],
    current_epoch: ChainEpoch,
) -> Result<()> {
    validate_streams_state_structure(streams, accruals)?;
    // Persisted writes may be past due after null rounds or a quiet stream-engine interval.
    validate_projected_queue_inner(streams, current_epoch, None, false, None)?;
    Ok(())
}

/// Validates every persisted invariant except the aggregate weight envelope.
fn validate_streams_state_structure(
    streams: &StreamsState,
    accruals: &[StreamAccrual],
) -> Result<()> {
    validate_streams_state_structure_without_weights(streams, accruals)?;
    for stream in &streams.streams {
        validate_weight_record(&stream.weight)?;
    }
    Ok(())
}

fn validate_streams_state_structure_without_weights(
    streams: &StreamsState,
    accruals: &[StreamAccrual],
) -> Result<()> {
    validate_award_state_structure(streams)?;
    if !(accruals.windows(2).all(|rows| rows[0].id < rows[1].id)) {
        return Err(anyhow::anyhow!("explicit-stream accruals are not ordered"));
    }

    let explicit_ids: BTreeSet<_> = streams
        .streams
        .iter()
        .filter(|stream| stream.distribution.is_some())
        .map(|stream| stream.id)
        .collect();
    let accrual_ids: BTreeSet<_> = accruals.iter().map(|row| row.id).collect();
    if explicit_ids != accrual_ids {
        return Err(anyhow::anyhow!(
            "explicit-stream accrual IDs do not match live explicit streams"
        ));
    }
    for accrual in accruals {
        if accrual.amount.is_negative() {
            return Err(anyhow::anyhow!("explicit-stream accrual {} is negative", accrual.id));
        }
        // Exact accrual-ID equality above proves this is a live explicit stream.
        let distribution = streams
            .streams
            .iter()
            .find(|stream| stream.id == accrual.id)
            .and_then(|stream| stream.distribution.as_ref())
            .expect("explicit-stream accrual IDs matched explicit streams");
        validate_period_claims(distribution, &accrual.amount)?;
    }
    Ok(())
}

/// Validates award-critical state that is independent of weights and accrual accounting.
pub(crate) fn validate_award_state_structure(streams: &StreamsState) -> Result<()> {
    validate_pending_queue(&streams.pending_writes, None)?;
    validate_stream_configuration_without_weights(&streams.streams)?;
    if !(streams.tombstones.windows(2).all(|rows| rows[0].id < rows[1].id)) {
        return Err(anyhow::anyhow!("tombstones are not ordered"));
    }

    let live_ids: BTreeSet<_> = streams.streams.iter().map(|stream| stream.id).collect();
    let tombstone_ids: BTreeSet<_> =
        streams.tombstones.iter().map(|tombstone| tombstone.id).collect();
    if !(live_ids.is_disjoint(&tombstone_ids)) {
        return Err(anyhow::anyhow!("a stream ID is live and tombstoned"));
    }
    for tombstone in &streams.tombstones {
        if tombstone.id == 0 {
            return Err(anyhow::anyhow!("stream ID 0 is reserved"));
        }
        if tombstone.payable.is_empty() {
            return Err(anyhow::anyhow!("tombstone {} is empty", tombstone.id));
        }
        validate_amount_rows(&tombstone.payable, "tombstone payable")?;
    }
    for write in &streams.pending_writes {
        if write.op == PendingWriteOp::RegisterStream {
            // Pending-queue shape validation requires IDs on every per-stream operation.
            let id = write.id.expect("validated per-stream pending call has an ID");
            if live_ids.contains(&id) || tombstone_ids.contains(&id) {
                return Err(anyhow::anyhow!("pending registration reuses stream ID {id}"));
            }
        }
    }
    validate_tombstone_capacity(streams)?;
    Ok(())
}

fn validate_distribution_init(distribution: &Option<DistributionInit>) -> Result<()> {
    if let Some(distribution) = distribution {
        validate_id_address(&distribution.writer, "distribution writer")?;
        validate_shares(&distribution.shares)?;
    }
    Ok(())
}

fn validate_id_address(address: &Address, label: &str) -> Result<()> {
    if address.protocol() != Protocol::ID {
        return Err(anyhow::anyhow!("{label} {address} is not an ID address"));
    }
    Ok(())
}

fn validate_tombstone_capacity(streams: &StreamsState) -> Result<()> {
    let mut rows: usize = streams.tombstones.iter().map(|tombstone| tombstone.payable.len()).sum();
    for write in &streams.pending_writes {
        if write.op != PendingWriteOp::RemoveStream {
            continue;
        }
        let id = write.id.expect("validated removal has a stream ID");
        rows += streams
            .streams
            .iter()
            .find(|stream| stream.id == id)
            .and_then(|stream| stream.distribution.as_ref())
            .map_or(MAX_RECIPIENTS, |distribution| {
                MAX_RECIPIENTS.max(recipient_union_len(&distribution.payable, &distribution.shares))
            });
    }
    if rows > MAX_TOMBSTONE_ROWS {
        return Err(anyhow::anyhow!(
            "tombstone row reservation {rows} exceeds maximum {MAX_TOMBSTONE_ROWS}"
        ));
    }
    Ok(())
}

fn recipient_union_len(payable: &[RecipientAmount], shares: &[RecipientShare]) -> usize {
    let mut payable_idx = 0;
    let mut shares_idx = 0;
    let mut count = 0;
    while payable_idx < payable.len() && shares_idx < shares.len() {
        count += 1;
        match payable[payable_idx].recipient.cmp(&shares[shares_idx].recipient) {
            std::cmp::Ordering::Less => payable_idx += 1,
            std::cmp::Ordering::Equal => {
                payable_idx += 1;
                shares_idx += 1;
            }
            std::cmp::Ordering::Greater => shares_idx += 1,
        }
    }
    count + payable.len() - payable_idx + shares.len() - shares_idx
}

/// Applies one captured call. Supplying accruals enables settlement and liability effects.
fn apply_pending_transition(
    state: &mut StreamsState,
    accruals: Option<&mut Vec<StreamAccrual>>,
    write: &PendingWrite,
) -> Result<TokenAmount> {
    match write.op {
        PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords => {
            if write.id.is_some() {
                return Err(anyhow::anyhow!("schedule-wide call has a stream ID"));
            }
            let payload: WeightRecordsPayload = write.payload.deserialize()?;
            validate_weight_updates(&payload.updates)?;
            for update in payload.updates {
                let stream = state
                    .streams
                    .iter_mut()
                    .find(|stream| stream.id == update.id)
                    .ok_or_else(|| anyhow::anyhow!("stream {} not found", update.id))?;
                stream.weight = update.weight;
            }
            Ok(TokenAmount::zero())
        }
        PendingWriteOp::RegisterStream => {
            let id =
                write.id.ok_or_else(|| anyhow::anyhow!("RegisterStream call has no stream ID"))?;
            if id == 0 {
                return Err(anyhow::anyhow!("stream ID 0 is reserved"));
            }
            let payload: RegisterStreamPayload = write.payload.deserialize()?;
            validate_weight_record(&payload.weight)?;
            validate_distribution_init(&payload.distribution)?;
            if state.streams.iter().any(|stream| stream.id == id) {
                return Err(anyhow::anyhow!("stream ID {id} is already registered"));
            }
            if state.tombstones.iter().any(|tombstone| tombstone.id == id) {
                return Err(anyhow::anyhow!("stream ID {id} is tombstoned"));
            }
            let distribution = payload.distribution.map(|distribution| ExplicitDistribution {
                writer: distribution.writer,
                shares: distribution.shares,
                payable: Vec::new(),
                claimed_period: Vec::new(),
            });
            if distribution.is_some()
                && let Some(accruals) = accruals
            {
                accruals.push(StreamAccrual { id, amount: TokenAmount::zero() });
                accruals.sort_by_key(|row| row.id);
            }
            state.streams.push(Stream { id, weight: payload.weight, distribution });
            state.streams.sort_by_key(|stream| stream.id);
            Ok(TokenAmount::zero())
        }
        PendingWriteOp::RemoveStream => {
            let id =
                write.id.ok_or_else(|| anyhow::anyhow!("RemoveStream call has no stream ID"))?;
            match accruals {
                Some(accruals) => {
                    remove_stream(&mut state.streams, &mut state.tombstones, accruals, id)
                }
                None => {
                    let idx = state
                        .streams
                        .iter()
                        .position(|stream| stream.id == id)
                        .ok_or_else(|| anyhow::anyhow!("stream {id} not found"))?;
                    state.streams.remove(idx);
                    Ok(TokenAmount::zero())
                }
            }
        }
        PendingWriteOp::SetDistribution => {
            let id =
                write.id.ok_or_else(|| anyhow::anyhow!("SetDistribution call has no stream ID"))?;
            let payload: SetDistributionPayload = write.payload.deserialize()?;
            match accruals {
                Some(accruals) => replace_writer(&mut state.streams, accruals, id, payload.writer),
                None => {
                    let stream = state
                        .streams
                        .iter_mut()
                        .find(|stream| stream.id == id)
                        .ok_or_else(|| anyhow::anyhow!("stream {id} not found"))?;
                    let distribution = stream
                        .distribution
                        .as_mut()
                        .ok_or_else(|| anyhow::anyhow!("stream {id} is implicit"))?;
                    distribution.writer = payload.writer;
                    Ok(TokenAmount::zero())
                }
            }
        }
    }
}

fn remove_stream(
    streams: &mut Vec<Stream>,
    tombstones: &mut Vec<Tombstone>,
    accruals: &mut Vec<StreamAccrual>,
    id: StreamId,
) -> Result<TokenAmount> {
    let idx = streams
        .iter()
        .position(|stream| stream.id == id)
        .ok_or_else(|| anyhow::anyhow!("stream {id} not found"))?;
    let mut stream = streams.remove(idx);
    let Some(distribution) = stream.distribution.as_mut() else {
        return Ok(TokenAmount::zero());
    };
    // Removing without the accrual row would orphan an unknown explicit-stream liability.
    let accrual_idx = accruals
        .iter()
        .position(|row| row.id == id)
        .ok_or_else(|| anyhow::anyhow!("missing accrual for stream {id}"))?;
    let accrual = accruals.remove(accrual_idx);
    let burn = settle_period(distribution, &accrual.amount)?;
    if !distribution.payable.is_empty() {
        if tombstones.iter().any(|tombstone| tombstone.id == id) {
            return Err(anyhow::anyhow!("stream ID {id} is already tombstoned"));
        }
        tombstones.push(Tombstone { id, payable: std::mem::take(&mut distribution.payable) });
        tombstones.sort_by_key(|tombstone| tombstone.id);
    }
    Ok(burn)
}

fn replace_writer(
    streams: &mut [Stream],
    accruals: &mut [StreamAccrual],
    id: StreamId,
    writer: Address,
) -> Result<TokenAmount> {
    let stream = streams
        .iter_mut()
        .find(|stream| stream.id == id)
        .ok_or_else(|| anyhow::anyhow!("stream {id} not found"))?;
    let distribution =
        stream.distribution.as_mut().ok_or_else(|| anyhow::anyhow!("stream {id} is implicit"))?;
    // Changing writers without the accrual row could reassign an unknown explicit-stream liability.
    let accrual = accruals
        .iter_mut()
        .find(|row| row.id == id)
        .ok_or_else(|| anyhow::anyhow!("missing accrual for stream {id}"))?;
    let burn = settle_period(distribution, &accrual.amount)?;
    accrual.amount = TokenAmount::zero();
    distribution.writer = writer;
    Ok(burn)
}

#[cfg(test)]
#[path = "streams_test.rs"]
mod tests;
