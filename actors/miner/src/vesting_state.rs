// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use std::iter;

use cid::Cid;
use fil_actors_runtime::{ActorError, AsActorError};
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::CborStore;
use fvm_ipld_encoding::serde::{Deserialize, Serialize};
use fvm_ipld_encoding::tuple::*;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use fvm_shared::error::ExitCode;
use itertools::{EitherOrBoth, Itertools, PeekingNext};
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
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(transparent)]
pub struct VestingFunds(Option<VestingFundsInner>);

#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone)]
struct VestingFundsInner {
    // The "next" batch of vesting funds.
    head: VestingFund,
    // The rest of the vesting funds, if any.
    tail: Cid, // Vec<VestingFund>
}

/// Take vested funds from the passed iterator. This assumes the iterator returns `VestingFund`s in
/// epoch order.
fn take_vested(
    iter: &mut impl PeekingNext<Item = VestingFund>,
    current_epoch: ChainEpoch,
) -> TokenAmount {
    iter.peeking_take_while(|fund| fund.epoch < current_epoch).map(|f| f.amount).sum()
}

impl VestingFunds {
    pub fn new() -> Self {
        Self(None)
    }

    pub fn load(&self, store: &impl Blockstore) -> Result<Vec<VestingFund>, ActorError> {
        let Some(this) = &self.0 else { return Ok(Vec::new()) };
        let mut funds: Vec<_> = store
            .get_cbor(&this.tail)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to load the vesting funds")?
            .context_code(ExitCode::USR_ILLEGAL_STATE, "missing vesting funds state")?;

        // NOTE: we allow head to be drawn down to zero through fees/penalties. However, when
        // inspecting the vesting table, we never want to see a "zero" entry so we skip it in
        // that case.
        if this.head.amount.is_positive() {
            funds.insert(0, this.head.clone());
        }
        Ok(funds)
    }

    fn save(
        &mut self,
        store: &impl Blockstore,
        funds: impl IntoIterator<Item = VestingFund>,
    ) -> Result<(), ActorError> {
        let mut funds = funds.into_iter();
        let Some(head) = funds.next() else {
            self.0 = None;
            return Ok(());
        };

        let tail = store
            .put_cbor(&funds.collect_vec(), Code::Blake2b256)
            .context_code(ExitCode::USR_ILLEGAL_STATE, "failed to store the vesting funds")?;

        self.0 = Some(VestingFundsInner { head, tail });
        Ok(())
    }

    fn can_vest(&self, current_epoch: ChainEpoch) -> bool {
        self.0.as_ref().map(|v| v.head.epoch < current_epoch).unwrap_or(false)
    }

    pub fn unlock_vested_funds(
        &mut self,
        store: &impl Blockstore,
        current_epoch: ChainEpoch,
    ) -> Result<TokenAmount, ActorError> {
        if !self.can_vest(current_epoch) {
            return Ok(TokenAmount::zero());
        }
        let mut funds = self.load(store)?.into_iter().peekable();
        let unlocked = take_vested(&mut funds, current_epoch);
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
        // Quantization is aligned with the beginning of the next 0th or 23rd deadline, whichever
        // comes first, and funds vest the epoch _after_ the quantized epoch (vesting_epoch <
        // current_epoch).
        //
        // This means that:
        //
        // 1. Vesting funds will become available to withdraw the first epoch after the start of the
        //    0th or 23rd deadline.
        // 2. Vesting funds won't automatically vest in cron until the next deadline (the 1st or the
        //    24th).

        let vest_begin = current_epoch + spec.initial_delay; // Nothing unlocks here, this is just the start of the clock.
        let quant = QuantSpec { unit: spec.quantization, offset: proving_period_start };

        let mut vested_so_far = TokenAmount::zero();
        let mut epoch = vest_begin;

        // Create an iterator for the vesting schedule we're going to "join" with the current
        // vesting schedule.
        let new_funds = iter::from_fn(|| {
            if vested_so_far >= *vesting_sum {
                return None;
            }

            epoch += spec.step_duration;

            let vest_epoch = quant.quantize_up(epoch);

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
        let mut combined_funds = old_funds
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
        let unlocked = take_vested(&mut combined_funds, current_epoch);

        // Write back the new value.
        self.save(store, combined_funds)?;

        Ok(unlocked)
    }

    /// Unlock all vested (first return value) then unlock unvested funds up to at most the
    /// specified target.
    pub fn unlock_vested_and_unvested_funds(
        &mut self,
        store: &impl Blockstore,
        current_epoch: ChainEpoch,
        target: &TokenAmount,
    ) -> Result<
        (
            TokenAmount, // automatic vested
            TokenAmount, // unlocked unvested
        ),
        ActorError,
    > {
        // If our inner value is None, there's nothing to vest.
        let Some(this) = &mut self.0 else {
            return Ok(Default::default());
        };

        let mut target = target.clone();

        // Fast path: take it out of the head and don't touch the tail.
        if this.head.epoch >= current_epoch && this.head.amount >= target {
            this.head.amount -= &target;
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
