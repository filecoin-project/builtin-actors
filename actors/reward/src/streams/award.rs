//! Dividing one block reward among the streams, and measuring what f02 still owes.
//!
//! FIP-0118 2.4.3, with the counters it moves specified in 2.5:
//!
//! ```text
//! AwardBlockReward(miner, penalty, gas_reward, win_count):
//!     require balance >= gas_reward
//!     load the streams block            // absent block or store error aborts
//!     computed_BR = this_epoch_reward * win_count / 5
//!     if the block is undecodable, or its stream, tombstone or queue
//!        structure or its accounting is invalid:
//!         no_award()
//!     projection = project valid due writes and cancellation-stranded drops
//!     fold_dust = projection's fold dust
//!     liability =
//!         sum projected live (accrued - sum claimed_period + sum payable)
//!         + sum projected tombstone payable
//!     if balance <= gas_reward + liability + fold_dust:
//!         no_award()
//!     BR = min(computed_BR, balance - gas_reward - liability - fold_dust)
//!     if BR < 0:                        // only a negative this_epoch_reward
//!         no_award()
//!     evaluated = ComputeWeight for every active stream
//!     if any record violates 0 <= floor <= v_start <= cap <= DENOM
//!        or sum evaluated > DENOM:
//!         no_award()
//!     commit projection
//!     miner_reward = 0
//!     allocated = 0
//!     burn = 0
//!     for each active stream s:                     // in list order
//!         portion = floor(evaluated[s] * BR / DENOM)
//!         allocated += portion
//!         if s.distribution is IMPLICIT:
//!             miner_reward += portion
//!         else:
//!             share_total = sum s.distribution.shares // stored; f099 absent
//!             accrue = floor(portion * share_total / DENOM)
//!             accrued[s.id] += accrue
//!             total_explicit_minted += accrue
//!             burn += portion - accrue
//!     burn += BR - allocated
//!     send(f099, burn + fold_dust); total_burn_minted += burn
//!     total_minted_reward += BR
//!     pay miner_reward + gas_reward to winning miner; penalties as today
//!
//! no_award():
//!     pay gas_reward and apply penalty as today; return without state change
//! ```
//!
//! Every award is one of those two outcomes: `no_award`, which pays the gas reward alone and
//! leaves the state as it stands, or the full split above.
//! [`plan_award`] chooses between them, in the order written above, and
//! `Actor::award_block_reward` applies what it chose and performs the sends. The pieces it calls:
//! - [`Ledger::allocate`] is the per-stream loop
//! - [`Ledger::accrue`] adds the resulting portions to the inline accrual rows
//! - [`Ledger::liability`] is the `liability` sum the reserve check subtracts, and the balance
//!   cover 2.5 requires of the counters this award moves

use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use log::{error, warn};
use num_traits::Zero;

use super::invariants::schedule_at;
use super::queue::ApplyResult;
use super::weights::compute_weight;
use super::{DENOM, Ledger, StreamAccrual, StreamId, StreamsState, accrual_mut};

/// One block reward split into its destinations.
///
/// This crosses Rust call boundaries only so doesn't need to be encodable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Allocation {
    pub miner: TokenAmount,
    /// This block's portion for each explicit stream, in stream order.
    pub portions: Vec<(StreamId, TokenAmount)>,
    pub burn: TokenAmount,
}

/// A block reward to mint, and everything the award owes once it is minted.
///
/// This crosses Rust call boundaries only so doesn't need to be encodable.
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
/// consumed is dropped rather than stored, so nothing is minted and no counter, stream record or
/// accrual moves.
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
    // A committed fold's dust is no longer owed to recipients but is not yet burnt, so it stays
    // reserved until the post-transaction send moves it.
    let reserve = gas_reward + &liability + &applied.fold_dust;
    if *balance <= reserve {
        warn!(
            "reward balance {balance} does not exceed gas {gas_reward}, explicit-stream \
             liabilities {liability} and pending dust {}; paying gas reward only",
            applied.fold_dust
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
    // The reserve bounds the reward from above alone, so a negative expected reward would reach
    // the split and allocate negative portions.
    if block_reward.is_negative() {
        error!("negative block reward {block_reward} at epoch {epoch}; paying gas reward only");
        return None;
    }
    if let Err(error) = schedule_at(&ledger.streams.streams, epoch) {
        warn!("invalid stream weights at epoch {epoch}: {error}; paying gas reward only");
        return None;
    }
    let allocation = ledger.allocate(epoch, &block_reward);
    ledger.accrue(&allocation.portions);
    Some((ledger, FullAward { block_reward, allocation, applied }))
}

impl Ledger {
    /// The value of the explicit-stream funds this ledger still holds.
    pub(crate) fn liability(&self) -> TokenAmount {
        explicit_liability(&self.streams, &self.accrued)
    }

    /// Splits one block reward across the active streams at `epoch`, in stream order.
    ///
    /// The caller has proved the schedule invariants at `epoch`, which is what keeps the split
    /// within the reward: each portion floors `weight * BR / DENOM` and the evaluated weights sum
    /// to at most `DENOM`, so the portions sum to at most `BR` and the residual burn covers the
    /// difference exactly.
    pub(crate) fn allocate(&self, epoch: ChainEpoch, block_reward: &TokenAmount) -> Allocation {
        let mut miner = TokenAmount::zero();
        let mut portions = Vec::with_capacity(self.streams.streams.len());
        let mut burn = TokenAmount::zero();
        let mut allocated = TokenAmount::zero();
        let denom = BigInt::from(DENOM);

        for stream in &self.streams.streams {
            let weight = compute_weight(&stream.weight, epoch);
            let mut portion = TokenAmount::from_atto(block_reward.atto() * weight / &denom);
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
    /// removal moves both together so we never have a mismatch.
    pub(crate) fn accrue(&mut self, portions: &[(StreamId, TokenAmount)]) {
        for (id, amount) in portions {
            let row = accrual_mut(&mut self.accrued, *id)
                .expect("accounting invariants: every explicit stream has an accrual row");
            row.amount += amount;
        }
    }
}

/// Explicit-stream funds held by f02. This is the unclaimed current-period earnings across the live
/// streams, plus every carried balance, live or tombstoned.
///
/// The caller's responsible for maintaining the accounting invariants (which every `Ledger` does,
/// and which the invariant checker proves before it measures a persisted state).
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
