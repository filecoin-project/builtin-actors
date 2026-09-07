//! Dividing one block reward among the streams and their recipients.
//!
//! FIP-0118 2.4.3, in the order [`plan_award`] runs it, with the counters it moves specified in
//! 2.5:
//!
//! ```text
//! AwardBlockReward(miner, penalty, gas_reward, win_count):
//!     require balance >= gas_reward
//!     load the streams block            // absent block or store error aborts
//!     computed_BR = this_epoch_reward * win_count / 5
//!     if the block is undecodable, or its stream or queue structure is invalid:
//!         no_award()
//!     apply the due writes, dropping the cancellation-stranded ones
//!     if balance <= gas_reward:         // nothing to mint; the writes stay queued
//!         no_award()
//!     BR = min(computed_BR, balance - gas_reward)
//!     if BR < 0:                        // only a negative this_epoch_reward
//!         no_award()
//!     evaluated = ComputeWeight for every active stream
//!     if any record violates 0 <= floor <= v_start <= cap <= DENOM
//!        or sum evaluated > DENOM:
//!         no_award()
//!     miner_reward = 0
//!     payouts = []                                  // (stream, recipient, amount)
//!     allocated = 0
//!     burn = 0
//!     for each active stream s:                     // in list order
//!         portion = floor(evaluated[s] * BR / DENOM)
//!         allocated += portion
//!         if s.distribution is IMPLICIT:
//!             miner_reward += portion
//!         else:
//!             paid = 0
//!             for each (wallet, share) in s.distribution.shares:   // stored; f099 absent
//!                 amount = floor(portion * share / DENOM)
//!                 payouts += (s.id, wallet, amount)
//!                 paid += amount
//!             total_explicit_minted += paid
//!             burn += portion - paid
//!     burn += BR - allocated
//!     total_burn_minted += burn
//!     total_minted_reward += BR
//!     store state
//!     require miner_reward + gas_reward + sum payouts + burn <= balance
//!     pay miner_reward + gas_reward to the winning miner; penalties as today;
//!         an unpaid miner reward joins the burn
//!     send each non-zero payout with method 0; an unpaid payout joins the burn
//!     send(f099, burn)                  // one burn send; an unsent burn is logged
//!
//! no_award():
//!     pay gas_reward and apply penalty as today; return without state change
//! ```
//!
//! Every award is one of those two outcomes: `no_award`, which pays the gas reward alone and
//! leaves the state as it stands, or the full split above. The due writes apply first and only the
//! second outcome stores a new ledger, so `no_award` leaves them queued for the next award.
//! [`plan_award`] chooses between them, in the order written above, and
//! `Actor::award_block_reward` applies what it chose and performs the sends. The pieces it calls:
//! - [`Ledger::apply_due`] applies the due writes
//! - [`schedule_at`] evaluates the weights and holds them within `DENOM`
//! - [`Ledger::allocate`] is the per-stream loop over those weights, down to one payout per
//!   stored recipient

use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use log::{error, warn};
use num_traits::Zero;

use super::Ledger;
use super::invariants::schedule_at;
use super::queue::ApplyResult;
use crate::state::{DENOM, StreamId};

/// One recipient's share of one stream's portion of a block reward.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Payout {
    pub stream: StreamId,
    pub recipient: Address,
    pub amount: TokenAmount,
}

/// One block reward split into its destinations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Allocation {
    pub miner: TokenAmount,
    /// One entry per stored recipient of every explicit stream, in stream order.
    pub payouts: Vec<Payout>,
    pub burn: TokenAmount,
}

/// A block reward to mint, and everything the award pays once it is minted.
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
    if *balance <= *gas_reward {
        warn!("reward balance {balance} does not exceed gas {gas_reward}; paying gas reward only");
        return None;
    }
    let available = balance - gas_reward;
    let block_reward = if *expected > available {
        warn!(
            "reward actor spendable balance {available} below block reward expected {expected}, \
             paying out spendable balance"
        );
        available
    } else {
        expected.clone()
    };
    // The balance caps the reward from above only, so without this guard a negative
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
    Some((ledger, FullAward { block_reward, allocation, applied }))
}

impl Ledger {
    /// Splits one block reward across the active streams, in stream order, under the weights
    /// [`schedule_at`] evaluated for this ledger, and each explicit stream's portion across its
    /// stored recipients.
    ///
    /// Those schedule invariants over the weights keep the split within the reward. Each portion
    /// floors `weight * BR / DENOM` and the weights sum to at most `DENOM`, so the portions sum to
    /// at most `BR`. Each payout floors `share * portion / DENOM` and the stored shares sum to at
    /// most `DENOM`, so a stream's payouts sum to at most its portion. The burn takes both
    /// differences exactly, so miner, payouts and burn together are `BR`.
    pub(crate) fn allocate(&self, evaluated: &[u64], block_reward: &TokenAmount) -> Allocation {
        debug_assert_eq!(
            evaluated.len(),
            self.streams.streams.len(),
            "the evaluated weights are one per stream"
        );
        let mut miner = TokenAmount::zero();
        let mut payouts = Vec::new();
        let mut burn = TokenAmount::zero();
        let mut allocated = TokenAmount::zero();
        let denom = BigInt::from(DENOM);

        for (stream, weight) in self.streams.streams.iter().zip(evaluated) {
            let portion = TokenAmount::from_atto(block_reward.atto() * *weight / &denom);
            allocated += &portion;
            if let Some(distribution) = stream.explicit() {
                let mut paid = TokenAmount::zero();
                for row in &distribution.shares {
                    let amount = TokenAmount::from_atto(portion.atto() * row.share / &denom);
                    paid += &amount;
                    payouts.push(Payout { stream: stream.id, recipient: row.recipient, amount });
                }
                burn += &portion - paid;
            } else {
                miner += portion;
            }
        }

        debug_assert!(allocated <= *block_reward, "the split exceeds the block reward");
        burn += block_reward - allocated;
        Allocation { miner, payouts, burn }
    }
}
