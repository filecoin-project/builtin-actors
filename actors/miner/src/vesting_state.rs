// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::iter;

use cid::Cid;
use fil_actors_runtime::{ActorError, AsActorError};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::tuple::*;
use fvm_ipld_encoding::CborStore;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use itertools::{EitherOrBoth, Itertools};
use multihash_codetable::Code;
use num_traits::Zero;

use super::{QuantSpec, VestSpec};

// Represents miner funds that will vest at the given epoch.
#[derive(Default, Debug, Serialize_tuple, Deserialize_tuple, Clone)]
pub struct VestingFund {
    pub epoch: ChainEpoch,
    pub amount: TokenAmount,
}

/// Represents the vesting table state for the miner. It's composed of a `head` (stored inline) and
/// a tail (referenced by CID).
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone)]
pub struct VestingFunds {
    // The "next" batch of vesting funds:
    // - If this is None, there are no vesting funds.
    // - All batches in `tail` are guaranteed to vest after this batch.
    // - This batch _can_ be empty (have a zero amount) if we've burnt through it (fees &
    //   penalties).
    head: Option<VestingFund>,
    // The rest of the vesting funds, if any.
    tail: Cid, // Vec<VestingFund>
}

impl VestingFunds {
    pub fn new(store: &impl Blockstore) -> Result<Self, ActorError> {
        let tail = store
            .put_cbor(&Vec::<VestingFund>::new(), Code::Blake2b256)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to construct vesting funds")?;
        Ok(Self { head: None, tail })
    }

    pub fn load(
        &self,
        store: &impl Blockstore,
    ) -> Result<impl Iterator<Item = VestingFund>, ActorError> {
        // NOTE: we allow head to be drawn down to zero through fees/penalties. However, when
        // inspecting the vesting table, we never want to see a "zero" entry so we skip it in that case.
        // We don't set it to "none" in that case because "none" means that we have _no_ vesting funds.
        let head = self.head.as_ref().filter(|h| h.amount.is_positive()).cloned();
        let tail: Vec<_> = store
            .get_cbor(&self.tail)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load the vesting funds")?
            .context_code(ExitCode::USR_ILLEGAL_STATE, "missing vesting funds state")?;
        Ok(itertools::chain(head, tail))
    }

    fn save(
        &mut self,
        store: &impl Blockstore,
        funds: impl IntoIterator<Item = VestingFund>,
    ) -> Result<(), ActorError> {
        let mut funds = funds.into_iter();
        let head = funds.next();
        let tail = funds.collect_vec();

        self.tail = store
            .put_cbor(&tail, Code::Blake2b256)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to store the vesting funds")?;
        // We do this second just in case the first operation fails to try to maintain consistent
        // state.
        self.head = head;
        Ok(())
    }

    pub fn can_vest(&self, current_epoch: ChainEpoch) -> bool {
        matches!(&self.head, Some(VestingFund { epoch, .. }) if *epoch < current_epoch)
    }

    pub fn unlock_vested_funds(
        &mut self,
        store: &impl Blockstore,
        current_epoch: ChainEpoch,
    ) -> Result<TokenAmount, ActorError> {
        if !self.can_vest(current_epoch) {
            return Ok(TokenAmount::zero());
        }
        let mut funds = self.load(store)?.peekable();
        let unlocked = funds
            .peeking_take_while(|fund| fund.epoch < current_epoch)
            .map(|f| f.amount)
            .sum::<TokenAmount>();
        self.save(store, funds)?;
        Ok(unlocked)
    }

    // Adds locked funds and unlocks everything that has already vested.
    pub fn add_locked_funds(
        &mut self,
        store: &impl Blockstore,
        current_epoch: ChainEpoch,
        vesting_sum: &TokenAmount,
        proving_period_start: ChainEpoch,
        spec: &VestSpec,
    ) -> Result<TokenAmount, ActorError> {
        // Quantization is aligned with when regular cron will be invoked, in the last epoch of deadlines.
        let vest_begin = current_epoch + spec.initial_delay; // Nothing unlocks here, this is just the start of the clock.
        let mut vested_so_far = TokenAmount::zero();

        let mut epoch = vest_begin;

        // Create an iterator for the vesting schedule we're going to "join" with the current
        // vesting schedule.
        let new_funds = iter::from_fn(|| {
            if vested_so_far >= *vesting_sum {
                return None;
            }

            epoch += spec.step_duration;

            let vest_epoch = QuantSpec { unit: spec.quantization, offset: proving_period_start }
                .quantize_up(epoch);

            let elapsed = vest_epoch - vest_begin;
            let target_vest = if elapsed < spec.vest_period {
                // Linear vesting
                (vesting_sum * elapsed).div_floor(spec.vest_period)
            } else {
                vesting_sum.clone()
            };

            let vest_this_time = &target_vest - &vested_so_far;
            vested_so_far = target_vest;

            Some(VestingFund { epoch: vest_epoch, amount: vest_this_time })
        });

        let old_funds = self.load(store)?;

        // Fill back in the funds array, merging existing and new schedule.
        let mut new_funds = old_funds
            .into_iter()
            .merge_join_by(new_funds, |a, b| a.epoch.cmp(&b.epoch))
            .map(|item| match item {
                EitherOrBoth::Left(a) => a,
                EitherOrBoth::Right(b) => b,
                EitherOrBoth::Both(a, b) => {
                    VestingFund { epoch: a.epoch, amount: a.amount + b.amount }
                }
            })
            .peekable();

        // Take any unlocked funds.
        let unlocked = new_funds
            .peeking_take_while(|fund| fund.epoch < current_epoch)
            .map(|f| f.amount)
            .sum::<TokenAmount>();

        // Write back the new value.
        self.save(store, new_funds.collect::<Vec<_>>())?;

        Ok(unlocked)
    }

    /// Unlock all vested (first return value) then unlock unvested funds up to at most the
    /// specified target.
    pub fn unlock_vested_and_unvested_funds(
        &mut self,
        store: &impl Blockstore,
        current_epoch: ChainEpoch,
        target: &TokenAmount,
    ) -> Result<(TokenAmount, TokenAmount), ActorError> {
        let mut target = target.clone();
        // Fast path: take it out of the head and don't touch the tail.
        let Some(head) = &mut self.head else {
            // If head is none, we have nothing to unlock.
            return Ok(Default::default());
        };
        if head.epoch >= current_epoch && head.amount >= target {
            head.amount -= &target;
            return Ok((TokenAmount::zero(), target));
        }

        // Slow path, take it out of the tail.

        let mut unvested = TokenAmount::zero();
        let mut vested = TokenAmount::zero();

        let mut funds = itertools::put_back(self.load(store)?);
        while let Some(mut vf) = funds.next() {
            // already vested
            if vf.epoch < current_epoch {
                vested += vf.amount;
                continue;
            }

            // take all
            if vf.amount < target {
                target -= &vf.amount;
                unvested += &vf.amount;
                continue;
            }

            // take some and stop.
            unvested += &target;
            vf.amount -= &target;
            funds.put_back(vf);
            break;
        }
        self.save(store, funds)?;

        Ok((vested, unvested))
    }
}
