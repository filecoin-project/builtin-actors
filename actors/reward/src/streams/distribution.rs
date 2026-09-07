//! Admitting an explicit stream's share map, and the three operations that reshape a stream
//! prospectively (FIP-0118 2.4.4 and 2.4.6).
//!
//! The shapes it reads and writes are in [`crate::state`]: [`ExplicitDistribution`] and its
//! [`RecipientShare`] map.
//!
//! An installed map is in force from the next award, and every award pays under the map it finds.
//!
//! FIP-0118 2.4.4, `SetShares`:
//!
//! ```text
//! SetShares(id, new_map):     // designated writer only; never queued
//!     apply the due writes               // a due SetDistribution changes who the writer is
//!     require caller == the stream's designated writer
//!     resolve each recipient to an ID address, rejecting on failure
//!     require sum new_map wire shares == DENOM, every share positive
//!     reject a repeated recipient, except f099, which may appear more than once
//!     strip f099 rows from new_map
//!     install new_map                                // in force from the next award
//! ```
//!
//! [`admit_shares`] is the validation and stripping; [`Ledger::set_shares`] installs what it
//! returns. The caller check and the address resolution are the actor layer's, because they need
//! the runtime.
//!
//! The other two operations are queued rather than immediate, and the queue applies them through
//! [`Ledger::remove_stream`], which takes a stream out of the schedule, and
//! [`Ledger::replace_writer`], which points one at a new designated writer while its share map
//! stays as it is.

use std::collections::BTreeSet;

use anyhow::{Result, ensure};
use fil_actors_runtime::{BURNT_FUNDS_ACTOR_ADDR, REWARD_ACTOR_ADDR};
use fvm_shared::address::{Address, Protocol};

use super::Ledger;
use super::queue::Stranded;
use crate::state::{DENOM, MAX_RECIPIENTS, RecipientShare, Stream, StreamId};
use crate::types::DistributionInit;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ShareForm {
    Wire,
    Stored,
}

/// Checks every share row the same way for both forms. The count stays within the maximum, a
/// stored map ascends by recipient, recipients are ID addresses, shares are positive, f02 is not a
/// recipient, and a recipient other than the burn sentinel appears once.
///
/// f02 is excluded because a send from f02 to itself moves nothing and reports no error, so such a
/// row would silently keep the reward.
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
        ensure!(row.recipient != REWARD_ACTOR_ADDR, "the reward actor is not a share recipient");
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
/// remains for storage. A stripped row leaves the stored shares summing below `DENOM`, and the
/// unpaid remainder of the stream's portion burns at every award.
pub(crate) fn admit_shares(mut shares: Vec<RecipientShare>) -> Result<Vec<RecipientShare>> {
    validate_shares(&shares)?;
    shares.retain(|row| row.recipient != BURNT_FUNDS_ACTOR_ADDR);
    shares.sort_by_key(|row| row.recipient);
    Ok(shares)
}

impl Ledger {
    /// Installs an explicit stream's next recipient share map, in force from the next award.
    pub(crate) fn set_shares(&mut self, id: StreamId, shares: Vec<RecipientShare>) -> Result<()> {
        self.streams_dirty = true;
        // Admit the incoming map, which is what turns caller rows into storable ones.
        let shares = admit_shares(shares)?;
        ensure!(self.streams.has_stream(id), "stream {id} not found");
        let Some(distribution) = self.streams.stream_mut(id).and_then(Stream::explicit_mut) else {
            return Err(anyhow::anyhow!("stream {id} is implicit"));
        };
        distribution.shares = shares;
        Ok(())
    }

    /// Takes a live stream out of the schedule. The next award pays under the streams that remain.
    pub(super) fn remove_stream(&mut self, id: StreamId) -> Result<(), Stranded> {
        self.streams.take_stream(id).ok_or(Stranded::MissingStream(id))?;
        Ok(())
    }

    /// Points an explicit stream at a new designated writer, keeping its share map.
    pub(super) fn replace_writer(&mut self, id: StreamId, writer: Address) -> Result<(), Stranded> {
        let stream = self.streams.stream_mut(id).ok_or(Stranded::MissingStream(id))?;
        let distribution = stream.explicit_mut().ok_or(Stranded::NotExplicit(id))?;
        distribution.writer = writer;
        Ok(())
    }
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
