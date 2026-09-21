//! Dividing one block reward among the streams, and measuring what f02 still owes.
//!
//! An award has two outcomes. `no_award` pays the miner its gas reward and leaves the state as
//! it stands. The full award divides one block reward and stores the result.
//!
//! 1. Load and check the streams block. Undecodable or invalid: `no_award`.
//! 2. Apply the due writes (timelock elapsed). A stranded one is dropped.
//! 3. Reserve the gas reward, what the explicit streams are owed, and the fold dust.
//! 4. `BR = min(this_epoch_reward * win_count / 5, balance - reserve)`. Reserve not covered, or
//!    `BR < 0`: `no_award`.
//! 5. Evaluate every stream's weight at this epoch. Out of band, or sum over `DENOM`: `no_award`.
//! 6. Split `BR` by weight. Implicit portions pay the miner, explicit portions accrue to their
//!    streams, rounding and the stripped f099 share burn.
//! 7. Check the result, store it, move the counters.
//! 8. Send `miner_reward + gas_reward` to the miner and `burn + fold dust` to f099.
//!
//! `no_award` stores nothing, so the due writes stay queued for the next award. [`plan_award`]
//! chooses the outcome; `Actor::award_block_reward` stores, moves the counters and sends. The
//! pieces that [`plan_award`] calls are:
//! - [`Ledger::apply_due`] applies the due writes and reports their dust
//! - [`schedule_at`] evaluates the weights and holds them within `DENOM`
//! - [`Ledger::allocate`] is the per-stream split
//! - [`Ledger::accrue`] adds the explicit portions to the inline accrual rows
//! - [`Ledger::liability`] is what the explicit streams are owed, the reserve above

use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use log::{error, warn};
use num_traits::Zero;

use super::Ledger;
use super::invariants::schedule_at;
use super::queue::ApplyResult;
use crate::state::{DENOM, StreamAccrual, StreamId, StreamsState};

/// One block reward split into its destinations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Allocation {
    pub miner: TokenAmount,
    /// This block's portion for each explicit stream, in stream order.
    pub portions: Vec<(StreamId, TokenAmount)>,
    pub burn: TokenAmount,
}

/// A block reward to mint, and everything the award owes once it is minted.
#[derive(Debug)]
pub(crate) struct FullAward {
    pub block_reward: TokenAmount,
    pub allocation: Allocation,
    /// The due writes this award committed, for the actor layer's events.
    pub applied: ApplyResult,
}

/// Chooses between the two outcomes of an award at `epoch`.
///
/// `None` pays the gas reward alone and is for the various unexpected error cases. The ledger it
/// consumed is dropped rather than stored, so the state stands exactly as the award found it.
/// `Some` hands back the ledger to store alongside the split to pay out.
pub(crate) fn plan_award(
    mut ledger: Ledger,
    epoch: ChainEpoch,
    balance: &TokenAmount,
    gas_reward: &TokenAmount,
    expected: &TokenAmount,
) -> Option<(Ledger, FullAward)> {
    let applied = ledger.apply_due(epoch);
    let liability = ledger.liability();
    // A committed fold's dust belongs to f099, and the send that does that comes _after_ the
    // transaction, so the reserve still holds it here.
    let fold_dust = applied.fold_dust();
    let reserve = gas_reward + &liability + &fold_dust;
    if *balance <= reserve {
        warn!(
            "reward balance {balance} does not exceed gas {gas_reward}, explicit-stream \
             liabilities {liability} and pending dust {fold_dust}; paying gas reward only"
        );
        return None;
    }
    let available = balance - reserve;
    let block_reward = if *expected > available {
        warn!(
            "reward actor spendable balance {available} below block reward expected {expected}, \
             paying out spendable balance"
        );
        available
    } else {
        expected.clone()
    };
    // The reserve caps the reward from above only, so without this guard a negative
    // this_epoch_reward reaches the split and allocates negative portions.
    if block_reward.is_negative() {
        error!("negative block reward {block_reward} at epoch {epoch}; paying gas reward only");
        return None;
    }
    let evaluated = match schedule_at(&ledger.streams.streams, epoch) {
        Ok(evaluated) => evaluated,
        Err(error) => {
            warn!("invalid stream weights at epoch {epoch}: {error}; paying gas reward only");
            return None;
        }
    };
    let allocation = ledger.allocate(&evaluated, &block_reward);
    ledger.accrue(&allocation.portions);
    Some((ledger, FullAward { block_reward, allocation, applied }))
}

impl Ledger {
    /// The value of the explicit-stream funds this ledger still holds.
    pub(crate) fn liability(&self) -> TokenAmount {
        explicit_liability(&self.streams, &self.accrued)
    }

    /// Splits one block reward across the active streams, in stream order, under the weights
    /// [`schedule_at`] evaluated for this ledger.
    ///
    /// Those schedule invariants over the weights keeps the split within the reward. Each portion
    /// floors `weight * BR / DENOM` and the weights sum to at most `DENOM`, so the portions sum to
    /// at most `BR` and the residual burn covers the difference exactly.
    pub(crate) fn allocate(&self, evaluated: &[u64], block_reward: &TokenAmount) -> Allocation {
        debug_assert_eq!(
            evaluated.len(),
            self.streams.streams.len(),
            "the evaluated weights are one per stream"
        );
        let mut miner = TokenAmount::zero();
        let mut portions = Vec::with_capacity(self.streams.streams.len());
        let mut burn = TokenAmount::zero();
        let mut allocated = TokenAmount::zero();
        let denom = BigInt::from(DENOM);

        for (stream, weight) in self.streams.streams.iter().zip(evaluated) {
            let mut portion = TokenAmount::from_atto(block_reward.atto() * *weight / &denom);
            allocated += &portion;
            if let Some(distribution) = stream.explicit() {
                let share_total = distribution.share_total();
                if share_total != DENOM {
                    let explicit_portion =
                        TokenAmount::from_atto(portion.atto() * share_total / &denom);
                    burn += &portion - &explicit_portion;
                    portion = explicit_portion;
                }
                portions.push((stream.id, portion));
            } else {
                miner += portion;
            }
        }

        debug_assert!(allocated <= *block_reward, "the split exceeds the block reward");
        burn += block_reward - allocated;
        Allocation { miner, portions, burn }
    }

    /// Adds an award's explicit-stream portions to their matching accrual rows.
    ///
    /// The portions are one per explicit stream in the state they were allocated from, which the
    /// accounting invariants pairs one to one with the accrual rows. A projected registration or
    /// removal moves both together, so they stay paired.
    pub(crate) fn accrue(&mut self, portions: &[(StreamId, TokenAmount)]) {
        for (id, amount) in portions {
            let accrual = self
                .accrual_mut(*id)
                .expect("accounting invariants: every explicit stream has an accrual row");
            *accrual += amount;
        }
    }
}

/// Explicit-stream funds held by f02. This is the unclaimed current-period earnings across the live
/// streams, plus every carried balance, live or tombstoned.
///
/// This is the slice-shaped form of `Ledger::liability`, for a caller holding no `Ledger`. The
/// invariant checker measures state that may be corrupt. The caller's responsible for maintaining
/// the accounting invariants.
pub fn explicit_liability(streams: &StreamsState, accrued: &[StreamAccrual]) -> TokenAmount {
    let mut total = TokenAmount::zero();

    for accrual in accrued {
        let distribution = streams
            .explicit(accrual.id)
            .expect("accounting invariants: an accrual row belongs to one live explicit stream");
        let claimed: TokenAmount = distribution.claimed_period.iter().map(|row| &row.amount).sum();
        total += &accrual.amount - claimed;
        total += distribution.payable.iter().map(|row| &row.amount).sum::<TokenAmount>();
    }
    for tombstone in &streams.tombstones {
        total += tombstone.payable.iter().map(|row| &row.amount).sum::<TokenAmount>();
    }
    total
}
