//! Explicit-stream recipient accounting: the share map, the period fold, and claims.
//!
//! Field order and enum discriminants are wire format for the types declared here.
//!
//! A period is the interval between two `SetShares` calls on one stream. f02 knows nothing of
//! quarters and imposes no cadence; installing a new map first closes the current period under
//! the outgoing one, which is what makes a share change strictly prospective.
//!
//! FIP-0118 2.4.4, `SetShares`:
//!
//! ```text
//! SetShares(id, new_map):     // designated writer only; never queued
//!     require caller == the stream's designated writer
//!     require sum new_map wire shares == DENOM, every share positive
//!     reject a repeated recipient, except f099, which may appear more
//!         than once
//!     resolve each recipient to an ID address, rejecting on failure
//!     strip f099 rows from new_map
//!     pool = accrued[id]
//!     share_total = sum OLD map shares              // stored; f099 absent
//!     for each (wallet, share) in the OLD map:
//!         earned = floor(share * pool / share_total)
//!         payable[wallet] += earned - claimed_period[wallet]
//!     residue = pool - sum earned               // rounding dust only
//!     send(f099, residue)                     // burned; neither counter moves
//!     accrued[id] = 0; clear claimed_period
//!     install new_map
//! ```
//!
//! FIP-0118 2.4.5, `Claim`:
//!
//! ```text
//! `Claim(id, wallets[]) -> amounts[]` is permissionless and batched. Each
//! wallet's entitlement is its live portion of the current period,
//! `floor(share * accrued[id] / stored_share_total)` minus what it has
//! already claimed this period, plus its payable balance from closed
//! periods; zero stored shares give no live entitlement. For a tombstoned
//! id, the payable balance alone applies. f02 records the claim
//! (bumping `claimed_period`, deleting the payable row), sends the wallet
//! its entitlement, and emits `claim-payout` (Section 2.4.9).
//! ```
//!
//! `fold` is the loop of the first block and returns the residue for the caller to burn;
//! `set_shares` wraps it with admission of the incoming map, and the queue calls it again for
//! removal and writer replacement (2.4.6). `claim` selects the live or tombstone arm of the
//! second block. The pool either divides is the stream's `StreamAccrual` row, passed in
//! because accruals live inline in root state rather than behind `streams_root`.

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;
use serde::{Deserialize, Serialize};

use super::invariants::validate_tombstone_capacity;
use super::queue::PendingWriteOp;
use super::{
    DENOM, MAX_PAYABLE_ROWS_PER_STREAM, MAX_RECIPIENTS, StreamAccrual, StreamId, StreamsState,
    accrual, accrual_mut,
};

/// One recipient entry in a share-map message and in persisted distribution state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RecipientShare {
    pub recipient: Address,
    pub share: u64,
}

/// Persisted allocation state for an explicit stream.
///
/// The accounting rows are actor-owned state, not caller-supplied share-map fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct ExplicitDistribution {
    /// Designated share writer, not a payee.
    pub writer: Address,
    /// Current recipient fractions for the open share period.
    pub shares: Vec<RecipientShare>,
    /// Unclaimed allocations carried from closed share periods.
    pub payable: RecipientTable,
    /// Amounts already claimed against the current period's gross accrual.
    pub claimed_period: RecipientTable,
}

impl ExplicitDistribution {
    /// The stored shares' total, which the structure invariant holds within `DENOM`.
    pub(super) fn share_total(&self) -> u64 {
        self.shares.iter().map(|row| row.share).sum()
    }
}

/// Persisted recipient balance in a live distribution or tombstone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RecipientAmount {
    pub recipient: Address,
    pub amount: TokenAmount,
}

/// A wallet-keyed balance table. Rows ascending by recipient, where none are zero.
///
/// The methods maintain that shape, which is the ordering 2.4.2 requires. `From` and
/// deserialization take whatever rows they are given, and persisted rows are checked by
/// `validate_amount_rows`. The encoded wire form is just the bare row array.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RecipientTable(Vec<RecipientAmount>);

impl RecipientTable {
    /// The recipient's balance, or zero when it holds no row.
    pub fn get(&self, recipient: &Address) -> TokenAmount {
        self.0
            .binary_search_by(|row| row.recipient.cmp(recipient))
            .map_or_else(|_| TokenAmount::zero(), |idx| self.0[idx].amount.clone())
    }

    /// Credits the recipient, inserting a row in order or accumulating onto its existing one.
    pub(super) fn add(&mut self, recipient: Address, amount: TokenAmount) {
        if amount.is_zero() {
            return;
        }
        match self.0.binary_search_by(|row| row.recipient.cmp(&recipient)) {
            Ok(idx) => self.0[idx].amount += amount,
            Err(idx) => self.0.insert(idx, RecipientAmount { recipient, amount }),
        }
    }

    /// Removes the recipient's row and returns its balance, or zero when it holds none.
    pub(super) fn take(&mut self, recipient: &Address) -> TokenAmount {
        self.0
            .binary_search_by(|row| row.recipient.cmp(recipient))
            .map_or_else(|_| TokenAmount::zero(), |idx| self.0.remove(idx).amount)
    }

    /// The number of rows this table would hold after folding a period under `shares`.
    pub(super) fn union_len(&self, shares: &[RecipientShare]) -> usize {
        let mut row_idx = 0;
        let mut share_idx = 0;
        let mut count = 0;
        while row_idx < self.0.len() && share_idx < shares.len() {
            count += 1;
            match self.0[row_idx].recipient.cmp(&shares[share_idx].recipient) {
                std::cmp::Ordering::Less => row_idx += 1,
                std::cmp::Ordering::Equal => {
                    row_idx += 1;
                    share_idx += 1;
                }
                std::cmp::Ordering::Greater => share_idx += 1,
            }
        }
        count + self.0.len() - row_idx + shares.len() - share_idx
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, RecipientAmount> {
        self.0.iter()
    }

    pub(super) fn clear(&mut self) {
        self.0.clear();
    }
}

impl From<Vec<RecipientAmount>> for RecipientTable {
    fn from(rows: Vec<RecipientAmount>) -> Self {
        RecipientTable(rows)
    }
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
pub(super) fn validate_stored_shares(shares: &[RecipientShare]) -> Result<()> {
    let total = validate_share_rows(shares, ShareForm::Stored)?;
    ensure!(total <= u128::from(DENOM), "stored shares sum to {total}, exceeds {DENOM}");
    Ok(())
}

/// Validates wire shares, strips burn sentinels, and orders persisted recipients.
pub(crate) fn normalize_shares(mut shares: Vec<RecipientShare>) -> Result<Vec<RecipientShare>> {
    validate_shares(&shares)?;
    shares.retain(|row| row.recipient != BURNT_FUNDS_ACTOR_ADDR);
    shares.sort_by_key(|row| row.recipient);
    Ok(shares)
}

/// Closes the current period, preserves unclaimed earnings, and installs new shares.
/// Returns indivisible rounding dust for burning.
pub(crate) fn set_shares(
    streams: &mut StreamsState,
    accruals: &mut [StreamAccrual],
    id: StreamId,
    shares: Vec<RecipientShare>,
) -> Result<TokenAmount> {
    let shares = normalize_shares(shares)?;
    let removal_pending =
        streams.pending_writes.iter().any(|write| write.op == PendingWriteOp::RemoveStream);
    let stream = streams.stream_mut(id).ok_or_else(|| anyhow::anyhow!("stream {id} not found"))?;
    let distribution =
        stream.explicit_mut().ok_or_else(|| anyhow::anyhow!("stream {id} is implicit"))?;
    let accrual = accrual_mut(accruals, id)
        .expect("accounting invariants: every explicit stream has an accrual row");

    let mut next_distribution = distribution.clone();
    let burn = fold(&mut next_distribution, &accrual.amount);
    let reserved_rows = next_distribution.payable.union_len(&shares);
    ensure!(
        reserved_rows <= MAX_PAYABLE_ROWS_PER_STREAM,
        "stream {id} payable row reservation {reserved_rows} exceeds maximum {MAX_PAYABLE_ROWS_PER_STREAM}"
    );
    next_distribution.shares = shares;

    *distribution = next_distribution;
    accrual.amount = TokenAmount::zero();
    // A pending removal has reserved tombstone rows for the map this fold leaves behind.
    if removal_pending {
        validate_tombstone_capacity(streams)?;
    }
    Ok(burn)
}

/// Claims live and carried earnings from either a registered stream or its tombstone.
pub(crate) fn claim(
    streams: &mut StreamsState,
    accruals: &[StreamAccrual],
    id: StreamId,
    wallets: &[Address],
) -> Result<ClaimResult> {
    if streams.has_stream(id) {
        let distribution =
            streams.explicit_mut(id).ok_or_else(|| anyhow::anyhow!("stream {id} is implicit"))?;
        let pool = accrual(accruals, id)
            .expect("accounting invariants: every explicit stream has an accrual row");
        Ok(claim_live(distribution, &pool.amount, wallets))
    } else if let Some(tombstone) = streams.tombstone_mut(id) {
        // A tombstone holds payable rows and nothing else, so a claim against it is a drain.
        let result: ClaimResult =
            wallets.iter().map(|wallet| tombstone.payable.take(wallet)).collect();
        let drained = tombstone.payable.is_empty();
        if drained {
            streams.take_tombstone(id);
        }
        Ok(result)
    } else {
        Ok(vec![TokenAmount::zero(); wallets.len()])
    }
}

/// Carries current-period earnings into payable balances and returns rounding dust.
pub(super) fn fold(distribution: &mut ExplicitDistribution, pool: &TokenAmount) -> TokenAmount {
    // Structure invariant: no stored share is zero, so a zero total means an empty map and no
    // division below.
    let denom = BigInt::from(distribution.share_total());
    let mut allocated = TokenAmount::zero();
    for share in &distribution.shares {
        let earned = TokenAmount::from_atto(pool.atto() * share.share / &denom);
        allocated += &earned;
        let claimed = distribution.claimed_period.get(&share.recipient);
        distribution.payable.add(share.recipient, earned - claimed);
    }
    distribution.claimed_period.clear();
    pool - allocated
}

fn claim_live(
    distribution: &mut ExplicitDistribution,
    pool: &TokenAmount,
    wallets: &[Address],
) -> ClaimResult {
    let share_total = distribution.share_total();
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
        let claimed = distribution.claimed_period.get(wallet);
        // Accounting invariant: every claimed-period row is within its recipient's earnings.
        debug_assert!(claimed <= earned);
        let live = earned - claimed;
        let payable = distribution.payable.get(wallet);
        let entitlement = &live + &payable;
        if entitlement.is_zero() {
            amounts.push(TokenAmount::zero());
            continue;
        }
        distribution.payable.take(wallet);
        distribution.claimed_period.add(*wallet, live);

        amounts.push(entitlement);
    }

    amounts
}

/// The relation between a stream's stored shares, its pool and what its recipients have already
/// claimed this period. The accounting invariant's per-stream half.
pub(super) fn validate_period_claims(
    distribution: &ExplicitDistribution,
    pool: &TokenAmount,
) -> Result<()> {
    // No stored share is zero, which makes the total a safe divisor below.
    validate_stored_shares(&distribution.shares)?;
    let share_total = distribution.share_total();
    ensure!(share_total != 0 || pool.is_zero(), "zero-share distribution has non-zero accrual");
    let denom = BigInt::from(share_total);
    for claimed in distribution.claimed_period.iter() {
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
    Ok(())
}

pub(super) fn validate_amount_rows(rows: &RecipientTable, label: &str) -> Result<()> {
    ensure!(
        rows.iter().is_sorted_by(|a, b| a.recipient < b.recipient),
        "{label} recipients are not ordered"
    );
    for row in rows.iter() {
        validate_id_address(&row.recipient, label)?;
        ensure!(row.amount > TokenAmount::zero(), "{label} amount is not positive");
    }
    Ok(())
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
