// Copyright 2019-2022 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

//! Every reward actor type that persists in its state, starting with the `State` block itself and
//! the StreamsState block that it references. Reward state mutates every epoch, but StreamsState
//! mutates much less often so we attempt to manage state churn by using the division.
//!
//! FIP-0118 section 2.4.2 defines the current layout as implemented here. It also defines
//! ordering: `streams` has unique ascending stream IDs; a share map has unique ascending
//! recipient IDs; and `pending_writes` is ordered by effective epoch, preserving admission order
//! at equal epochs.
//!
//! A stream's distribution is one of two kinds (2.4.1). `IMPLICIT` (the `None` arm) stores
//! nothing and pays the block winner, only the "consensus" stream is implicit. `EXPLICIT` (the
//! `Some` arm) carries a writer and a share map. The FIP-0118 migration pins consensus = 1 and
//! the service stream = 2, but f02 only knows and cares about the kind.

use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::CborStore;
use fvm_ipld_encoding::RawBytes;
use fvm_ipld_encoding::repr::*;
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

    /// Cumulative block-reward residual allocated to burn.
    pub total_burn_minted: TokenAmount,

    /// Cumulative block reward allocated to explicit-stream recipients.
    pub total_explicit_minted: TokenAmount,

    /// Hold applied to SWA writes. Construction leaves zero; the activation migration sets the
    /// operational value.
    pub swa_timelock_epochs: ChainEpoch,

    /// SWA actor authorized to manage stream configuration. Construction uses f00; the activation
    /// migration sets the operational address.
    pub swa_actor: Address,

    /// Offboarded StreamsState for active streams and queued-write state.
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
        // One implicit consensus stream at full weight: the whole reward reaches the miner
        // until a migration or the SWA installs a schedule.
        let streams = StreamsState {
            streams: vec![Stream {
                id: 1,
                weight: WeightRecord {
                    v_start: DENOM,
                    slope: 0,
                    t_start: 0,
                    floor: DENOM,
                    cap: DENOM,
                },
                distribution: None,
            }],
            ..Default::default()
        };
        let streams_root = store.put_cbor(&streams, Code::Blake2b256)?;
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
}

pub type StreamId = u64;

// These bound the work an award or a mutation can do, and raising any of them takes a FIP and a
// network upgrade that reshapes the affected structures for the new scale (FIP-0118 2.4.9).

/// The fixed point scale for weights and shares (FIP-0118 2.4.1(4)).
pub const DENOM: u64 = 1_000_000_000_000_000_000;
/// Streams the schedule can carry, counting the implicit consensus stream.
pub const MAX_STREAMS: usize = 8;
/// Recipients in one share map, each paid its own send at every award.
pub const MAX_RECIPIENTS: usize = 64;
/// Registration, removal and writer change, the three operations one stream can have queued.
const STREAM_SCOPED_PENDING_OPS: usize = 3;
/// `SetWeightRecords` and `StepWeightRecords`, which act on the whole schedule.
const SCHEDULE_WIDE_PENDING_OPS: usize = 2;
/// Entries in the pending-write queue, which is the slot space of every per-stream operation
/// across a full stream table plus the two schedule-wide slots. The queue enforces it as a plain
/// length check rather than deriving it from the slots in use.
pub const MAX_PENDING_WRITES: usize =
    MAX_STREAMS * STREAM_SCOPED_PENDING_OPS + SCHEDULE_WIDE_PENDING_OPS;

/// Stream state persisted as the block referenced by `State.streams_root`.
///
/// A `RegisterStream` for an ID that is live or already queued for registration is rejected at
/// admission by `ensure_stream_id_available`, and one whose ID has become live since strands at
/// application as `Stranded::StreamIdInUse`. An ID a removal has taken out of the table is free
/// to register again.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct StreamsState {
    /// Ordered by stream ID.
    pub streams: Vec<Stream>,
    /// Ordered by effective epoch; equal epochs retain queue position.
    pub pending_writes: Vec<PendingWrite>,
}

/// Lookups by stream ID. The tables are bounded, so a linear scan is fine.
impl StreamsState {
    pub(crate) fn stream(&self, id: StreamId) -> Option<&Stream> {
        self.streams.iter().find(|stream| stream.id == id)
    }

    pub(crate) fn stream_mut(&mut self, id: StreamId) -> Option<&mut Stream> {
        self.streams.iter_mut().find(|stream| stream.id == id)
    }

    /// The stored writer and share map of a live explicit stream.
    pub(crate) fn explicit(&self, id: StreamId) -> Option<&ExplicitDistribution> {
        self.stream(id).and_then(Stream::explicit)
    }

    pub(crate) fn has_stream(&self, id: StreamId) -> bool {
        self.stream(id).is_some()
    }

    /// Files a live stream, keeping the table ascending by stream ID.
    pub(crate) fn insert_stream(&mut self, stream: Stream) {
        let idx = self
            .streams
            .binary_search_by_key(&stream.id, |live| live.id)
            .expect_err("registration precondition: the stream ID is not live");
        self.streams.insert(idx, stream);
    }

    /// Removes the live stream with this ID and hands it over.
    pub(crate) fn take_stream(&mut self, id: StreamId) -> Option<Stream> {
        let idx = self.streams.iter().position(|stream| stream.id == id)?;
        Some(self.streams.remove(idx))
    }
}

/// A live stream persisted in `StreamsState`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct Stream {
    pub id: StreamId,
    pub weight: WeightRecord,
    /// None is the implicit consensus distribution; Some is an explicit distribution.
    pub distribution: Option<ExplicitDistribution>,
}

impl Stream {
    /// True for the consensus stream, whose portion pays the block winner directly.
    pub(crate) fn is_implicit(&self) -> bool {
        self.distribution.is_none()
    }

    /// The stored writer and share map, for an explicit stream.
    pub(crate) fn explicit(&self) -> Option<&ExplicitDistribution> {
        self.distribution.as_ref()
    }

    pub(crate) fn explicit_mut(&mut self) -> Option<&mut ExplicitDistribution> {
        self.distribution.as_mut()
    }
}

/// A clamped linear weight in `DENOM` fixed point.
///
/// Persisted in `Stream` and encoded in weight-management messages and deferred payloads.
/// Admitted records satisfy `floor <= v_start <= cap <= DENOM`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct WeightRecord {
    pub v_start: u64,
    pub slope: i64,
    pub t_start: ChainEpoch,
    pub floor: u64,
    pub cap: u64,
}

/// Persisted distribution for an explicit stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct ExplicitDistribution {
    /// Designated share writer, not a payee.
    pub writer: Address,
    /// The fraction of the stream's portion each recipient is paid at every award.
    pub shares: Vec<RecipientShare>,
}

/// One recipient entry in a share-map message and in persisted distribution state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct RecipientShare {
    pub recipient: Address,
    pub share: u64,
}

/// A deferred SWA operation persisted in `StreamsState`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize_tuple, Deserialize_tuple)]
pub struct PendingWrite {
    /// Per-stream target, or `None` for a schedule-wide call.
    pub id: Option<StreamId>,
    pub op: PendingWriteOp,
    /// Operation-specific CBOR tuple.
    pub payload: RawBytes,
    pub effective_epoch: ChainEpoch,
}

/// Stable operation tag persisted in `PendingWrite` and exposed by `CancelPending`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize_repr, Deserialize_repr)]
#[repr(u8)]
pub enum PendingWriteOp {
    SetWeightRecords = 0,
    /// Gate-originated weight update; unlike other operations, it cannot be cancelled.
    StepWeightRecords = 1,
    RegisterStream = 2,
    RemoveStream = 3,
    SetDistribution = 4,
}

impl PendingWriteOp {
    /// Schedule-wide operations act on the whole weight schedule and carry no stream ID.
    pub(crate) fn is_schedule_wide(self) -> bool {
        matches!(self, PendingWriteOp::SetWeightRecords | PendingWriteOp::StepWeightRecords)
    }
}
