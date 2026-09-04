//! Explicit-stream recipient accounting: the share map, the period fold, claims, and the two
//! lifecycle operations that fold (FIP-0118 2.4.4, 2.4.5 and 2.4.6).
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
//! [`Ledger::set_shares`] wraps it with admission of the incoming map. [`Ledger::claim`] selects
//! the live or tombstone arm of the second block. Both divide the same pool, the stream's
//! `StreamAccrual` row, which the ledger has beside the streams block (because accruals live
//! inline in root state rather than behind `streams_root`).
//!
//! Removing a stream and changing its writer (FIP-0118 2.4.6) both start with the same fold.
//! [`Ledger::remove_stream`] folds the open period, then moves whatever is still unpaid into a
//! tombstone under the stream's own ID. Claims against that ID keep working until the last row
//! is taken, then the tombstone deletes itself. The SWA should not reuse the ID after that, and
//! f02 rejects any reuse it can still see. [`Ledger::replace_writer`] folds, then points the
//! stream at its new writer; the share map stays as it is, so payments continue.
//!
//! We use two caps to keep those tables small: a live stream holds at most
//! `MAX_PAYABLE_ROWS_PER_STREAM` payable rows, checked by [`Ledger::set_shares`] on the map it is
//! about to install. All tombstones together hold at most `MAX_TOMBSTONE_ROWS`. A removal only
//! folds when it applies, so [`validate_tombstone_capacity`] reserves its rows at admission,
//! and checks the reservation again on every `SetShares` made while the removal is pending.

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use fil_actors_runtime::BURNT_FUNDS_ACTOR_ADDR;
use fvm_shared::address::{Address, Protocol};
use fvm_shared::bigint::BigInt;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::Ledger;
use super::invariants::validate_tombstone_capacity;
use super::queue::Stranded;
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

/// Checks every share row the same way for both forms. The count stays within the maximum, a
/// stored map ascends by recipient, recipients are ID addresses, shares are positive, and a
/// recipient other than the burn sentinel appears once.
fn validate_share_rows(shares: &[RecipientShare], form: ShareForm) -> Result<()> {
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
    }
    Ok(())
}

/// Validates a wire map whose sentinel-inclusive shares must sum to `DENOM`.
pub(super) fn validate_shares(shares: &[RecipientShare]) -> Result<()> {
    validate_share_rows(shares, ShareForm::Wire)?;
    let total: u128 = shares.iter().map(|row| u128::from(row.share)).sum();
    ensure!(total == u128::from(DENOM), "shares sum to {total}, expected {DENOM}");
    Ok(())
}

/// Validates a persisted map whose sentinel-free shares may sum below `DENOM`.
pub(super) fn validate_stored_shares(shares: &[RecipientShare]) -> Result<()> {
    validate_share_rows(shares, ShareForm::Stored)?;
    let total: u128 = shares.iter().map(|row| u128::from(row.share)).sum();
    ensure!(total <= u128::from(DENOM), "stored shares sum to {total}, exceeds {DENOM}");
    Ok(())
}

/// Admits a wire map by validating the shares, stripping the burn sentinels, and ordering what
/// remains for storage.
pub(crate) fn admit_shares(mut shares: Vec<RecipientShare>) -> Result<Vec<RecipientShare>> {
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
        // Admit the incoming map, which is what turns caller rows into storable ones.
        let shares = admit_shares(shares)?;
        // Read before the period borrow, for the tombstone recharge at the end.
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
        let mut next_pool = period.pool.clone();
        let burn = fold(&mut next_distribution, &mut next_pool);
        let reserved_rows = next_distribution.payable.union_len(&shares);
        ensure!(
            reserved_rows <= MAX_PAYABLE_ROWS_PER_STREAM,
            "stream {id} payable row reservation {reserved_rows} exceeds maximum {MAX_PAYABLE_ROWS_PER_STREAM}"
        );
        next_distribution.shares = shares;

        // Install the folded table and the new map together, opening the next period.
        *period.distribution = next_distribution;
        *period.pool = next_pool;
        // A pending removal reserved tombstone rows against the map this fold leaves behind, so
        // recharge that reservation.
        if removal_pending {
            validate_tombstone_capacity(&self.streams)?;
        }
        Ok(burn)
    }

    /// Removes a live stream, folding its closing period into a tombstone when anything is unpaid.
    pub(super) fn remove_stream(&mut self, id: StreamId) -> Result<TokenAmount, Stranded> {
        let mut stream = self.streams.take_stream(id).ok_or(Stranded::MissingStream(id))?;
        let Some(distribution) = stream.explicit_mut() else {
            return Ok(TokenAmount::zero());
        };
        let mut accrual = self
            .take_accrual(id)
            .expect("accounting invariants: every explicit stream has an accrual row");
        let burn = fold(distribution, &mut accrual);
        if !distribution.payable.is_empty() {
            self.streams.insert_tombstone(id, std::mem::take(&mut distribution.payable));
        }
        Ok(burn)
    }

    /// Closes an explicit stream's period and points it at a new designated writer.
    pub(super) fn replace_writer(
        &mut self,
        id: StreamId,
        writer: Address,
    ) -> Result<TokenAmount, Stranded> {
        if !self.streams.has_stream(id) {
            return Err(Stranded::MissingStream(id));
        }
        let Some(period) = self.period_mut(id) else {
            return Err(Stranded::NotExplicit(id));
        };
        let burn = fold(period.distribution, period.pool);
        period.distribution.writer = writer;
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
    /// addresses, so such a wallet keeps its position in the result and takes a zero entitlement
    /// there.
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
            // A live stream has a distribution and a pool, so we can claim from it.
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

/// Closes a period. Its earnings move into the payable balances, the pool it divided empties, and
/// the rounding dust the division left comes back for the caller to burn.
fn fold(distribution: &mut ExplicitDistribution, pool: &mut TokenAmount) -> TokenAmount {
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
    let dust = &*pool - allocated;
    *pool = TokenAmount::zero();
    dust
}

/// Claims live earnings for each wallet, returning the entitlements in request order, including
/// zero for any `None` wallet. We also update the distribution to reflect the claims.
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

        // Calculate the wallet's live entitlement, which is its share of the pool minus what it has
        // already claimed this period.
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

        // Add any payable balance from closed periods
        let payable = distribution.payable.get(wallet);

        // The wallet's entitlement is the sum of its live and payable balances.
        let entitlement = &live + &payable;
        if entitlement.is_zero() {
            amounts.push(TokenAmount::zero());
            continue;
        }

        // Update the distribution to reflect the claim, which is a drain of both the live and
        // payable balances. The payable row is deleted once it is drained.
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
