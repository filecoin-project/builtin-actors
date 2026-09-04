//! Explicit-stream recipient accounting: the share map, the period fold, and claims.
//!
//! The shapes it reads and writes are in [`crate::state`]: [`ExplicitDistribution`], its
//! [`RecipientShare`] map, and its [`RecipientTable`] balances.
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
//! [`Ledger::set_shares`] wraps it with admission of the incoming map, and the queue calls it
//! again for removal and writer replacement (2.4.6). [`Ledger::claim`] selects the live or
//! tombstone arm of the second block. Both divide the same pool, the stream's `StreamAccrual`
//! row, which the ledger carries beside the streams block because accruals live inline in root
//! state rather than behind `streams_root`.

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::Ledger;
use super::invariants::validate_tombstone_capacity;
use crate::state::{
    DENOM, ExplicitDistribution, MAX_PAYABLE_ROWS_PER_STREAM, MAX_RECIPIENTS, PendingWriteOp,
    RecipientShare, RecipientTable, StreamId,
};
use crate::types::DistributionInit;

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

impl Ledger {
    /// Closes the current period, preserves unclaimed earnings, and installs new shares.
    /// Returns indivisible rounding dust for burning.
    pub(crate) fn set_shares(
        &mut self,
        id: StreamId,
        shares: Vec<RecipientShare>,
    ) -> Result<TokenAmount> {
        self.streams_dirty = true;
        let shares = normalize_shares(shares)?;
        let removal_pending = self
            .streams
            .pending_writes
            .iter()
            .any(|write| write.op == PendingWriteOp::RemoveStream);
        ensure!(self.streams.has_stream(id), "stream {id} not found");
        let Some(period) = self.period_mut(id) else {
            return Err(anyhow::anyhow!("stream {id} is implicit"));
        };

        let mut next_distribution = period.distribution.clone();
        let burn = fold(&mut next_distribution, period.pool);
        let reserved_rows = next_distribution.payable.union_len(&shares);
        ensure!(
            reserved_rows <= MAX_PAYABLE_ROWS_PER_STREAM,
            "stream {id} payable row reservation {reserved_rows} exceeds maximum {MAX_PAYABLE_ROWS_PER_STREAM}"
        );
        next_distribution.shares = shares;

        *period.distribution = next_distribution;
        *period.pool = TokenAmount::zero();
        // A pending removal has reserved tombstone rows for the map this fold leaves behind.
        if removal_pending {
            validate_tombstone_capacity(&self.streams)?;
        }
        Ok(burn)
    }

    /// Claims live and carried earnings from either a registered stream or its tombstone.
    ///
    /// A claim always names a stream, and a removed stream is claimed under the same ID it had,
    /// because removal moves its unpaid rows into a tombstone filed under that ID. The tombstone
    /// deletes itself once its last row is claimed, and the ID then answers with zeros. A wallet
    /// owed by several streams claims from one stream at a time, live or tombstoned.
    ///
    /// A `None` wallet is one the actor layer could not resolve. Stored recipients are ID
    /// addresses, so such a wallet matches no row and we pass that through back to the caller with
    /// a zero entitlement.
    pub(crate) fn claim(
        &mut self,
        id: StreamId,
        wallets: &[Option<Address>],
    ) -> Result<ClaimResult> {
        self.streams_dirty = true;
        if self.streams.has_stream(id) {
            let Some(period) = self.period_mut(id) else {
                return Err(anyhow::anyhow!("stream {id} is implicit"));
            };
            Ok(claim_live(period.distribution, period.pool, wallets))
        } else if let Some(tombstone) = self.streams.tombstone_mut(id) {
            // A tombstone holds payable rows and nothing else, so a claim against it is a drain.
            let result: ClaimResult = wallets
                .iter()
                .map(|wallet| match wallet {
                    Some(wallet) => tombstone.payable.take(wallet),
                    None => TokenAmount::zero(),
                })
                .collect();
            let drained = tombstone.payable.is_empty();
            if drained {
                self.streams.take_tombstone(id);
            }
            Ok(result)
        } else {
            Ok(vec![TokenAmount::zero(); wallets.len()])
        }
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

/// Claims live earnings for each wallet, returning the entitlements in request order.
fn claim_live(
    distribution: &mut ExplicitDistribution,
    pool: &TokenAmount,
    wallets: &[Option<Address>],
) -> ClaimResult {
    let share_total = distribution.share_total();
    let denom = BigInt::from(share_total);
    let mut amounts = Vec::with_capacity(wallets.len());

    for wallet in wallets {
        let Some(wallet) = wallet else {
            // A `None` wallet is one the actor layer couldn't resolve, send it back with zero.
            amounts.push(TokenAmount::zero());
            continue;
        };
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
