// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::invariants::validate_tombstone_capacity;
use super::queue::PendingWriteOp;
use super::{
    DENOM, MAX_PAYABLE_ROWS_PER_STREAM, MAX_RECIPIENTS, StreamAccrual, StreamId, StreamsState,
};

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

/// Caller-supplied subset of a new explicit distribution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct DistributionInit {
    pub writer: Address,
    pub shares: Vec<RecipientShare>,
}

/// One claimed amount for each requested wallet, preserving request order.
pub(crate) type ClaimResult = Vec<TokenAmount>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShareForm {
    Wire,
    Stored,
}

fn validate_share_rows(shares: &[RecipientShare], form: ShareForm) -> Result<u128> {
    ensure!(
        shares.len() <= MAX_RECIPIENTS,
        "recipient count {} exceeds maximum {MAX_RECIPIENTS}",
        shares.len()
    );
    if form == ShareForm::Stored {
        ensure!(
            shares.is_sorted_by(|a, b| a.recipient < b.recipient),
            "stored share recipients are not ordered"
        );
    }

    let mut recipients = BTreeSet::new();
    let mut total = 0_u128;
    for row in shares {
        validate_id_address(&row.recipient, "share recipient")?;
        ensure!(row.share != 0, "share for recipient {} is zero", row.recipient);
        if row.recipient == BURNT_FUNDS_ACTOR_ADDR {
            ensure!(form == ShareForm::Wire, "burn sentinel persisted as a recipient");
        } else {
            ensure!(
                recipients.insert(row.recipient),
                "duplicate share recipient {}",
                row.recipient
            );
        }
        total += u128::from(row.share);
    }
    Ok(total)
}

/// Validates a wire map whose sentinel-inclusive shares must sum to `DENOM`.
pub(super) fn validate_shares(shares: &[RecipientShare]) -> Result<()> {
    let total = validate_share_rows(shares, ShareForm::Wire)?;
    ensure!(total == u128::from(DENOM), "shares sum to {total}, expected {DENOM}");
    Ok(())
}

/// Validates a persisted map whose sentinel-free shares may sum below `DENOM`.
pub(super) fn validate_stored_shares(shares: &[RecipientShare]) -> Result<u64> {
    let total = validate_share_rows(shares, ShareForm::Stored)?;
    ensure!(total <= u128::from(DENOM), "stored shares sum to {total}, exceeds {DENOM}");
    Ok(total as u64)
}

/// Validates wire shares, strips burn sentinels, and orders persisted recipients.
pub(crate) fn normalize_shares(mut shares: Vec<RecipientShare>) -> Result<Vec<RecipientShare>> {
    validate_shares(&shares)?;
    shares.retain(|row| row.recipient != BURNT_FUNDS_ACTOR_ADDR);
    shares.sort_by_key(|row| row.recipient);
    Ok(shares)
}

pub(super) fn stored_share_total(shares: &[RecipientShare]) -> Result<u64> {
    shares.iter().try_fold(0_u64, |total, row| {
        total.checked_add(row.share).ok_or_else(|| anyhow::anyhow!("stored shares overflow"))
    })
}

/// Closes the current period, preserves unclaimed earnings, and installs new shares.
/// Returns indivisible rounding dust for burning.
pub(crate) fn set_shares(
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
    accruals.clone_from_slice(&next_accruals);
    Ok(burn)
}

fn set_shares_inner(
    streams: &mut StreamsState,
    accruals: &mut [StreamAccrual],
    id: StreamId,
    shares: Vec<RecipientShare>,
) -> Result<TokenAmount> {
    let shares = normalize_shares(shares)?;
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

    let mut next_distribution = distribution.clone();
    let burn = settle_period(&mut next_distribution, &accrual.amount)?;
    let reserved_rows = recipient_union_len(&next_distribution.payable, &shares);
    ensure!(
        reserved_rows <= MAX_PAYABLE_ROWS_PER_STREAM,
        "stream {id} payable row reservation {reserved_rows} exceeds maximum {MAX_PAYABLE_ROWS_PER_STREAM}"
    );
    next_distribution.shares = shares;

    *distribution = next_distribution;
    accrual.amount = TokenAmount::zero();
    Ok(burn)
}

/// Claims live and carried earnings from either a registered stream or its tombstone.
pub(crate) fn claim(
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
pub(super) fn settle_period(
    distribution: &mut ExplicitDistribution,
    pool: &TokenAmount,
) -> Result<TokenAmount> {
    ensure!(!pool.is_negative(), "explicit-stream accrual is negative");
    let share_total = validate_period_claims(distribution, pool)?;

    let denom = BigInt::from(share_total);
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
    ensure!(!pool.is_negative(), "explicit-stream accrual is negative");
    let share_total = validate_period_claims(distribution, pool)?;
    let denom = BigInt::from(share_total);
    let mut amounts = Vec::with_capacity(wallets.len());

    for wallet in wallets {
        let share = distribution
            .shares
            .iter()
            .find(|row| row.recipient == *wallet)
            .map_or(0, |row| row.share);
        let earned = if share_total == 0 {
            TokenAmount::zero()
        } else {
            TokenAmount::from_atto(pool.atto() * share / &denom)
        };
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

pub(super) fn validate_period_claims(
    distribution: &ExplicitDistribution,
    pool: &TokenAmount,
) -> Result<u64> {
    validate_amount_rows(&distribution.payable, "payable")?;
    validate_amount_rows(&distribution.claimed_period, "claimed-period")?;
    let share_total = validate_stored_shares(&distribution.shares)?;
    ensure!(share_total != 0 || pool.is_zero(), "zero-share distribution has non-zero accrual");
    let denom = BigInt::from(share_total);
    for claimed in &distribution.claimed_period {
        let share =
            distribution
                .shares
                .iter()
                .find(|share| share.recipient == claimed.recipient)
                .ok_or_else(|| anyhow::anyhow!("claimed-period recipient is absent from shares"))?;
        let earned = TokenAmount::from_atto(pool.atto() * share.share / &denom);
        ensure!(
            claimed.amount <= earned,
            "claimed amount exceeds earnings for recipient {}",
            claimed.recipient
        );
    }
    Ok(share_total)
}

pub(super) fn validate_amount_rows(rows: &[RecipientAmount], label: &str) -> Result<()> {
    ensure!(
        rows.is_sorted_by(|a, b| a.recipient < b.recipient),
        "{label} recipients are not ordered"
    );
    for row in rows {
        validate_id_address(&row.recipient, label)?;
        ensure!(row.amount > TokenAmount::zero(), "{label} amount is not positive");
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

pub(super) fn validate_distribution_init(distribution: &Option<DistributionInit>) -> Result<()> {
    if let Some(distribution) = distribution {
        validate_id_address(&distribution.writer, "distribution writer")?;
        validate_stored_shares(&distribution.shares)?;
    }
    Ok(())
}

pub(super) fn validate_id_address(address: &Address, label: &str) -> Result<()> {
    ensure!(address.protocol() == Protocol::ID, "{label} {address} is not an ID address");
    Ok(())
}

pub(super) fn recipient_union_len(payable: &[RecipientAmount], shares: &[RecipientShare]) -> usize {
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
