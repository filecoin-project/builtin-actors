use fvm_ipld_encoding::to_vec;
use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::*;
use crate::state::*;
use crate::streams::award::Allocation;
use crate::streams::invariants::schedule_at;
use crate::streams::queue::Stranded;
use crate::streams::weights::compute_weight;
use crate::types::*;

mod award;
mod distribution;
mod invariants;
mod queue;
mod weights;

fn pct(value: u64) -> u64 {
    DENOM / 100 * value
}

fn constant_weight(value: u64) -> WeightRecord {
    WeightRecord { v_start: value, slope: 0, t_start: 0, floor: value, cap: value }
}

fn shares(rows: &[(u64, u64)]) -> Vec<RecipientShare> {
    rows.iter()
        .map(|(id, share)| RecipientShare { recipient: Address::new_id(*id), share: *share })
        .collect()
}

fn full_share_map(first_recipient: u64) -> Vec<RecipientShare> {
    let share = DENOM / MAX_RECIPIENTS as u64;
    (0..MAX_RECIPIENTS)
        .map(|idx| RecipientShare {
            recipient: Address::new_id(first_recipient + idx as u64),
            share,
        })
        .collect()
}

fn explicit(writer: u64, shares: Vec<RecipientShare>) -> ExplicitDistribution {
    ExplicitDistribution { writer: Address::new_id(writer), shares }
}

fn stream(id: StreamId, weight: u64, distribution: Option<ExplicitDistribution>) -> Stream {
    Stream { id, weight: constant_weight(weight), distribution }
}

fn base_state() -> StreamsState {
    StreamsState {
        streams: vec![
            stream(1, pct(60), None),
            stream(2, pct(20), Some(explicit(200, shares(&[(101, DENOM)])))),
        ],
        pending_writes: Vec::new(),
    }
}

/// A ledger over state the test built by hand, bypassing the invariants a load would run.
fn ledger(streams: &StreamsState) -> Ledger {
    Ledger { streams: streams.clone(), streams_dirty: false }
}

/// The queue operations as the actor drives them: load a ledger, act, keep what it holds only if
/// the call was admitted, since a rejected ledger is unspecified and the caller discards it.
fn admit(
    streams: &mut StreamsState,
    call: QueuedCall,
    epoch: ChainEpoch,
    timelock: ChainEpoch,
) -> anyhow::Result<PendingWrite> {
    let mut ledger = ledger(streams);
    let queued = ledger.admit(call.canonical(), epoch, timelock).cloned()?;
    *streams = ledger.streams;
    Ok(queued)
}

fn queue_weight_records(
    streams: &mut StreamsState,
    epoch: ChainEpoch,
    timelock: ChainEpoch,
    op: PendingWriteOp,
    updates: &[WeightRecordUpdate],
) -> anyhow::Result<PendingWrite> {
    let call = QueuedCall::Weights { op, updates: updates.to_vec() };
    admit(streams, call, epoch, timelock)
}

fn queue_register_stream(
    streams: &mut StreamsState,
    epoch: ChainEpoch,
    timelock: ChainEpoch,
    stream: Stream,
    activation: ChainEpoch,
) -> anyhow::Result<PendingWrite> {
    let call = QueuedCall::Register {
        id: stream.id,
        weight: stream.weight,
        distribution: stream.distribution.map(|distribution| DistributionInit {
            writer: distribution.writer,
            shares: distribution.shares,
        }),
        activation,
    };
    admit(streams, call, epoch, timelock)
}

fn queue_remove_stream(
    streams: &mut StreamsState,
    epoch: ChainEpoch,
    timelock: ChainEpoch,
    id: StreamId,
) -> anyhow::Result<PendingWrite> {
    admit(streams, QueuedCall::Remove { id }, epoch, timelock)
}

fn queue_set_distribution(
    streams: &mut StreamsState,
    epoch: ChainEpoch,
    timelock: ChainEpoch,
    id: StreamId,
    writer: Address,
) -> anyhow::Result<PendingWrite> {
    admit(streams, QueuedCall::SetDistribution { id, writer }, epoch, timelock)
}

/// The explicit share update as the actor drives it, keeping what the ledger holds only on
/// success.
fn set_shares(
    streams: &mut StreamsState,
    id: StreamId,
    shares: Vec<RecipientShare>,
) -> anyhow::Result<()> {
    let mut ledger = ledger(streams);
    ledger.set_shares(id, shares)?;
    *streams = ledger.streams;
    Ok(())
}

/// One due removal, driven straight through the transition the queue applies.
fn remove_stream(streams: &mut StreamsState, id: StreamId) -> Result<(), Stranded> {
    let mut ledger = ledger(streams);
    ledger.remove_stream(id)?;
    *streams = ledger.streams;
    Ok(())
}

/// One due writer change, driven straight through the transition the queue applies.
fn replace_writer(
    streams: &mut StreamsState,
    id: StreamId,
    writer: Address,
) -> Result<(), Stranded> {
    let mut ledger = ledger(streams);
    ledger.replace_writer(id, writer)?;
    *streams = ledger.streams;
    Ok(())
}

/// One block reward split across a bare stream table, which is all the split reads, under the
/// weights that table evaluates to at `epoch`.
fn allocate(streams: &[Stream], epoch: ChainEpoch, block_reward: &TokenAmount) -> Allocation {
    let table = StreamsState { streams: streams.to_vec(), ..Default::default() };
    let evaluated: Vec<u64> =
        streams.iter().map(|stream| compute_weight(&stream.weight, epoch)).collect();
    ledger(&table).allocate(&evaluated, block_reward)
}

/// Runs one award through `plan_award` with no gas reward and a balance equal to `reward`, so the
/// block reward is `reward`. Returns the streams the award leaves and its split. Asserts that the
/// reward equals miner plus payouts plus burn, and that an award which applies no queued write
/// leaves the streams block bytes unchanged.
fn full_award(
    streams: &StreamsState,
    epoch: ChainEpoch,
    reward: &TokenAmount,
) -> (StreamsState, FullAward) {
    let before = to_vec(streams).expect("the streams block encodes");
    let (ledger, award) = plan_award(ledger(streams), epoch, reward, &TokenAmount::zero(), reward)
        .expect("the award splits the reward under a valid schedule");
    let after = ledger.streams;

    assert_eq!(*reward, award.block_reward);
    assert_eq!(
        *reward,
        &award.allocation.miner + &paid(&award.allocation) + &award.allocation.burn,
        "the award splits the whole block reward"
    );
    if award.applied == ApplyResult::default() {
        assert_eq!(
            before,
            to_vec(&after).expect("the streams block encodes"),
            "an award that moves no queued write rewrote the streams block"
        );
    }
    (after, award)
}

/// What an allocation pays out across every stream and recipient.
fn paid(allocation: &Allocation) -> TokenAmount {
    allocation.payouts.iter().map(|payout| &payout.amount).sum()
}

/// One recipient's payout from one stream, which an award makes exactly once.
fn payout(allocation: &Allocation, stream: StreamId, recipient: u64) -> TokenAmount {
    let recipient = Address::new_id(recipient);
    let mut rows = allocation
        .payouts
        .iter()
        .filter(|payout| payout.stream == stream && payout.recipient == recipient);
    let amount = rows.next().map_or_else(TokenAmount::zero, |payout| payout.amount.clone());
    assert!(rows.next().is_none(), "one payout per stream and recipient");
    amount
}

fn apply_due_writes(streams: &mut StreamsState, epoch: ChainEpoch) -> ApplyResult {
    let mut ledger = ledger(streams);
    let result = ledger.apply_due(epoch);
    *streams = ledger.streams;
    result
}

/// The cancellation path as the actor drives it: resolve the slot, then empty it.
fn cancel(
    streams: &mut StreamsState,
    id: Option<StreamId>,
    op: PendingWriteOp,
) -> anyhow::Result<Option<PendingWrite>> {
    let slot = Slot::for_cancel(id, op)?;
    let mut ledger = ledger(streams);
    let removed = ledger.cancel(slot);
    *streams = ledger.streams;
    Ok(removed)
}

/// The three counters an award moves, and the miner's share, tracked across a run of awards.
#[derive(Default)]
struct SupplyTracker {
    total_minted: TokenAmount,
    total_burn: TokenAmount,
    total_explicit: TokenAmount,
    miner_total: TokenAmount,
}

impl SupplyTracker {
    /// Records one award and reports how many recipients it paid a non-zero amount.
    ///
    /// The caller has already applied the epoch's due writes, so this award applies none.
    fn award(&mut self, streams: &StreamsState, epoch: ChainEpoch, reward: TokenAmount) -> usize {
        let (_, award) = full_award(streams, epoch, &reward);
        assert_eq!(ApplyResult::default(), award.applied);
        let allocation = award.allocation;
        let paid = paid(&allocation);
        self.total_minted += &reward;
        self.total_burn += &allocation.burn;
        self.total_explicit += paid;
        self.miner_total += &allocation.miner;
        allocation.payouts.iter().filter(|payout| !payout.amount.is_zero()).count()
    }

    fn assert_invariants(&self) {
        assert_eq!(self.total_minted, &self.miner_total + &self.total_explicit + &self.total_burn);
    }
}
