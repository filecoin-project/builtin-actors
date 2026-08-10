// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::CborStore;
use fvm_ipld_encoding::tuple::*;
use fvm_shared::address::Address;
use fvm_shared::bigint::BigInt;
use fvm_shared::bigint::bigint_ser;
use fvm_shared::clock::{ChainEpoch, EPOCH_UNDEFINED};
use fvm_shared::econ::TokenAmount;
use fvm_shared::sector::StoragePower;
use lazy_static::lazy_static;
use multihash_codetable::Code;

use fil_actors_runtime::builtin::reward::smooth::{
    AlphaBetaFilter, DEFAULT_ALPHA, DEFAULT_BETA, FilterEstimate,
};

/// The unit of spacetime committed to the network
pub type Spacetime = BigInt;

use super::logic::*;
use super::streams::{StreamAccrual, StreamsState};

lazy_static! {
    /// 36.266260308195979333 FIL
    pub static ref INITIAL_REWARD_POSITION_ESTIMATE: TokenAmount = TokenAmount::from_atto(36266260308195979333u128);
    /// -1.0982489*10^-7 FIL per epoch.  Change of simple minted tokens between epochs 0 and 1.
    pub static ref INITIAL_REWARD_VELOCITY_ESTIMATE: TokenAmount = TokenAmount::from_atto(-109897758509i64);
}

/// Reward actor state
#[derive(Serialize_tuple, Deserialize_tuple, Debug, Clone)]
pub struct State {
    /// Target CumsumRealized needs to reach for EffectiveNetworkTime to increase
    /// Expressed in byte-epochs.
    #[serde(with = "bigint_ser")]
    pub cumsum_baseline: Spacetime,

    /// CumsumRealized is cumulative sum of network power capped by BaselinePower(epoch).
    /// Expressed in byte-epochs.
    #[serde(with = "bigint_ser")]
    pub cumsum_realized: Spacetime,

    /// Ceiling of real effective network time `theta` based on
    /// CumsumBaselinePower(theta) == CumsumRealizedPower
    /// Theta captures the notion of how much the network has progressed in its baseline
    /// and in advancing network time.
    pub effective_network_time: ChainEpoch,

    /// EffectiveBaselinePower is the baseline power at the EffectiveNetworkTime epoch.
    #[serde(with = "bigint_ser")]
    pub effective_baseline_power: StoragePower,

    /// The reward to be paid in per WinCount to block producers.
    /// The actual reward total paid out depends on the number of winners in any round.
    /// This value is recomputed every non-null epoch and used in the next non-null epoch.
    pub this_epoch_reward: TokenAmount,
    /// Smoothed `this_epoch_reward`.
    pub this_epoch_reward_smoothed: FilterEstimate,

    /// The baseline power the network is targeting at st.Epoch.
    #[serde(with = "bigint_ser")]
    pub this_epoch_baseline_power: StoragePower,

    /// Epoch tracks for which epoch the Reward was computed.
    pub epoch: ChainEpoch,

    /// Total FIL minted through block rewards.
    pub total_minted_reward: TokenAmount,

    /// Cumulative block-reward residual sent to the burnt funds actor.
    pub total_burn_minted: TokenAmount,

    /// Cumulative block reward accrued to explicit service streams.
    pub total_explicit_minted: TokenAmount,

    /// Current-period accrual for each explicit stream, ordered by stream ID.
    pub accrued: Vec<StreamAccrual>,

    /// Hold applied to SWA writes. Set only by the activation migration.
    pub swa_timelock_epochs: ChainEpoch,

    /// SWA actor authorized to manage stream configuration. Set only by the activation migration.
    pub swa_actor: Address,

    /// Offboarded stream, tombstone, and queued-write state.
    pub streams_root: Cid,
}

impl Default for State {
    fn default() -> Self {
        Self {
            cumsum_baseline: Default::default(),
            cumsum_realized: Default::default(),
            effective_network_time: Default::default(),
            effective_baseline_power: Default::default(),
            this_epoch_reward: Default::default(),
            this_epoch_reward_smoothed: Default::default(),
            this_epoch_baseline_power: Default::default(),
            epoch: Default::default(),
            total_minted_reward: Default::default(),
            total_burn_minted: Default::default(),
            total_explicit_minted: Default::default(),
            accrued: Default::default(),
            swa_timelock_epochs: Default::default(),
            swa_actor: Address::new_id(0),
            streams_root: Default::default(),
        }
    }
}

impl State {
    pub fn new<BS: Blockstore>(
        store: &BS,
        curr_realized_power: StoragePower,
    ) -> anyhow::Result<Self> {
        let streams_root = store.put_cbor(&StreamsState::default(), Code::Blake2b256)?;
        let mut st = Self {
            effective_baseline_power: BASELINE_INITIAL_VALUE.clone(),
            this_epoch_baseline_power: INIT_BASELINE_POWER.clone(),
            epoch: EPOCH_UNDEFINED,
            this_epoch_reward_smoothed: FilterEstimate::new(
                INITIAL_REWARD_POSITION_ESTIMATE.atto().clone(),
                INITIAL_REWARD_VELOCITY_ESTIMATE.atto().clone(),
            ),
            streams_root,
            ..Default::default()
        };
        st.update_to_next_epoch_with_reward(&curr_realized_power);

        Ok(st)
    }

    /// Takes in current realized power and updates internal state
    /// Used for update of internal state during null rounds
    pub(super) fn update_to_next_epoch(&mut self, curr_realized_power: &StoragePower) {
        self.epoch += 1;
        self.this_epoch_baseline_power = baseline_power_from_prev(&self.this_epoch_baseline_power);
        let capped_realized_power =
            std::cmp::min(&self.this_epoch_baseline_power, curr_realized_power);
        self.cumsum_realized += capped_realized_power;

        while self.cumsum_realized > self.cumsum_baseline {
            self.effective_network_time += 1;
            self.effective_baseline_power =
                baseline_power_from_prev(&self.effective_baseline_power);
            self.cumsum_baseline += &self.effective_baseline_power;
        }
    }

    /// Takes in a current realized power for a reward epoch and computes
    /// and updates reward state to track reward for the next epoch
    pub(super) fn update_to_next_epoch_with_reward(&mut self, curr_realized_power: &StoragePower) {
        let prev_reward_theta = compute_r_theta(
            self.effective_network_time,
            &self.effective_baseline_power,
            &self.cumsum_realized,
            &self.cumsum_baseline,
        );
        self.update_to_next_epoch(curr_realized_power);
        let curr_reward_theta = compute_r_theta(
            self.effective_network_time,
            &self.effective_baseline_power,
            &self.cumsum_realized,
            &self.cumsum_baseline,
        );

        self.this_epoch_reward = compute_reward(self.epoch, prev_reward_theta, curr_reward_theta);
    }

    pub(super) fn update_smoothed_estimates(&mut self, delta: ChainEpoch) {
        let filter_reward =
            AlphaBetaFilter::load(&self.this_epoch_reward_smoothed, &DEFAULT_ALPHA, &DEFAULT_BETA);
        self.this_epoch_reward_smoothed =
            filter_reward.next_estimate(self.this_epoch_reward.atto(), delta);
    }

    pub fn into_total_minted_reward(self) -> TokenAmount {
        self.total_minted_reward
    }
}
