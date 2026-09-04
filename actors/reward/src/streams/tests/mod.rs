use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::*;
use crate::streams::award::Allocation;
use crate::streams::invariants::schedule_at;

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
    ExplicitDistribution {
        writer: Address::new_id(writer),
        shares,
        payable: RecipientTable::default(),
        claimed_period: RecipientTable::default(),
    }
}

fn stream(id: StreamId, weight: u64, distribution: Option<ExplicitDistribution>) -> Stream {
    Stream { id, weight: constant_weight(weight), distribution }
}

fn base_state() -> (StreamsState, Vec<StreamAccrual>) {
    (
        StreamsState {
            streams: vec![
                stream(1, pct(60), None),
                stream(2, pct(20), Some(explicit(200, shares(&[(101, DENOM)])))),
            ],
            tombstones: Vec::new(),
            pending_writes: Vec::new(),
        },
        vec![StreamAccrual { id: 2, amount: TokenAmount::zero() }],
    )
}

fn tombstone(id: StreamId, first_recipient: u64, rows: usize) -> Tombstone {
    Tombstone {
        id,
        payable: (0..rows)
            .map(|offset| RecipientAmount {
                recipient: Address::new_id(first_recipient + offset as u64),
                amount: TokenAmount::from_atto(1),
            })
            .collect::<Vec<_>>()
            .into(),
    }
}

/// A ledger over state the test built by hand, bypassing the invariants a load would run.
///
/// Wherever the operation under test reads an accrual row, the state given here has to satisfy
/// those invariants anyway: the engine's accrual lookups are `expect`s, so a live explicit stream
/// without its row panics rather than returning the rejection the test is looking for.
fn ledger(streams: &StreamsState, accruals: &[StreamAccrual]) -> Ledger {
    Ledger { streams: streams.clone(), accrued: accruals.to_vec(), streams_dirty: false }
}

/// The queue operations as the actor drives them: load a ledger, act, keep what it holds only if
/// the call was admitted, since a rejected ledger is unspecified and the caller discards it.
///
/// Admission never changes the accrual rows, so these hand them back untouched and take a slice.
fn admit(
    streams: &mut StreamsState,
    accruals: &[StreamAccrual],
    call: QueuedCall,
    epoch: ChainEpoch,
    timelock: ChainEpoch,
) -> anyhow::Result<PendingWrite> {
    let mut ledger = ledger(streams, accruals);
    let queued = ledger.admit(call, epoch, timelock).cloned()?;
    *streams = ledger.streams;
    Ok(queued)
}

fn queue_weight_records(
    streams: &mut StreamsState,
    accruals: &[StreamAccrual],
    epoch: ChainEpoch,
    timelock: ChainEpoch,
    op: PendingWriteOp,
    updates: &[WeightRecordUpdate],
) -> anyhow::Result<PendingWrite> {
    let call = QueuedCall::Weights { op, updates: updates.to_vec() };
    admit(streams, accruals, call, epoch, timelock)
}

fn queue_register_stream(
    streams: &mut StreamsState,
    accruals: &[StreamAccrual],
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
    admit(streams, accruals, call, epoch, timelock)
}

fn queue_remove_stream(
    streams: &mut StreamsState,
    accruals: &[StreamAccrual],
    epoch: ChainEpoch,
    timelock: ChainEpoch,
    id: StreamId,
) -> anyhow::Result<PendingWrite> {
    admit(streams, accruals, QueuedCall::Remove { id }, epoch, timelock)
}

fn queue_set_distribution(
    streams: &mut StreamsState,
    accruals: &[StreamAccrual],
    epoch: ChainEpoch,
    timelock: ChainEpoch,
    id: StreamId,
    writer: Address,
) -> anyhow::Result<PendingWrite> {
    admit(streams, accruals, QueuedCall::SetDistribution { id, writer }, epoch, timelock)
}

/// The explicit share update as the actor drives it, keeping what the ledger holds only on
/// success.
fn set_shares(
    streams: &mut StreamsState,
    accruals: &mut Vec<StreamAccrual>,
    id: StreamId,
    shares: Vec<RecipientShare>,
) -> anyhow::Result<TokenAmount> {
    let mut ledger = ledger(streams, accruals);
    let dust = ledger.set_shares(id, shares)?;
    *streams = ledger.streams;
    *accruals = ledger.accrued;
    Ok(dust)
}

/// A claim over wallets the actor layer has already resolved.
fn claim(
    streams: &mut StreamsState,
    accruals: &[StreamAccrual],
    id: StreamId,
    wallets: &[Address],
) -> anyhow::Result<Vec<TokenAmount>> {
    let mut ledger = ledger(streams, accruals);
    let resolved: Vec<Option<Address>> = wallets.iter().copied().map(Some).collect();
    let amounts = ledger.claim(id, &resolved)?;
    *streams = ledger.streams;
    Ok(amounts)
}

/// One block reward split across a bare stream table, which is all the split reads.
fn allocate(streams: &[Stream], epoch: ChainEpoch, block_reward: &TokenAmount) -> Allocation {
    let table = StreamsState { streams: streams.to_vec(), ..Default::default() };
    ledger(&table, &[]).allocate(epoch, block_reward)
}

/// The award crediting its portions, which changes no row's presence and so no length.
fn accrue(accruals: &mut [StreamAccrual], portions: &[(StreamId, TokenAmount)]) {
    let mut ledger = ledger(&StreamsState::default(), accruals);
    ledger.accrue(portions);
    accruals.clone_from_slice(&ledger.accrued);
}

/// The accrual rows an award's portions amount to from an empty start.
fn accruals_of(portions: &[(StreamId, TokenAmount)]) -> Vec<StreamAccrual> {
    portions.iter().map(|(id, amount)| StreamAccrual { id: *id, amount: amount.clone() }).collect()
}

fn apply_due_writes(
    streams: &mut StreamsState,
    accruals: &mut Vec<StreamAccrual>,
    epoch: ChainEpoch,
) -> ApplyResult {
    let mut ledger = ledger(streams, accruals);
    let result = ledger.apply_due(epoch);
    *streams = ledger.streams;
    *accruals = ledger.accrued;
    result
}

/// The cancellation path as the actor drives it: resolve the slot, then empty it.
fn cancel(
    streams: &mut StreamsState,
    id: Option<StreamId>,
    op: PendingWriteOp,
) -> anyhow::Result<Option<PendingWrite>> {
    let slot = Slot::for_cancel(id, op)?;
    let mut ledger = ledger(streams, &[]);
    let removed = ledger.cancel(slot);
    *streams = ledger.streams;
    Ok(removed)
}

fn amount(rows: &RecipientTable, recipient: u64) -> TokenAmount {
    rows.get(&Address::new_id(recipient))
}

fn explicit_liabilities(streams: &StreamsState, accruals: &[StreamAccrual]) -> TokenAmount {
    ledger(streams, accruals).liability()
}

fn assert_explicit_conserved(
    gross: &TokenAmount,
    paid: &TokenAmount,
    burned: &TokenAmount,
    streams: &StreamsState,
    accruals: &[StreamAccrual],
) {
    let mut accounted = paid.clone();
    accounted += burned;
    accounted += explicit_liabilities(streams, accruals);
    assert_eq!(*gross, accounted);
}

#[derive(Default)]
struct SupplyTracker {
    total_minted: TokenAmount,
    total_burn: TokenAmount,
    total_explicit: TokenAmount,
    total_dust: TokenAmount,
    f099_balance: TokenAmount,
    actor_balance: TokenAmount,
}

impl SupplyTracker {
    fn award(
        &mut self,
        streams: &StreamsState,
        accruals: &mut [StreamAccrual],
        epoch: ChainEpoch,
        reward: TokenAmount,
    ) {
        schedule_at(&streams.streams, epoch).expect("valid randomized state entered degradation");
        let allocation = allocate(&streams.streams, epoch, &reward);
        let explicit = allocation
            .portions
            .iter()
            .fold(TokenAmount::zero(), |total, (_, amount)| total + amount);
        accrue(accruals, &allocation.portions);
        self.total_minted += &reward;
        self.total_burn += &allocation.burn;
        self.total_explicit += &explicit;
        self.f099_balance += allocation.burn;
        self.actor_balance += explicit;
    }

    fn burn_dust(&mut self, dust: TokenAmount) {
        self.actor_balance -= &dust;
        self.total_dust += &dust;
        self.f099_balance += dust;
    }

    fn pay_claim(&mut self, result: &[TokenAmount]) {
        let paid = result.iter().fold(TokenAmount::zero(), |total, amount| total + amount);
        self.actor_balance -= paid;
    }

    fn assert_invariants(&self, streams: &StreamsState, accruals: &[StreamAccrual]) {
        let liabilities = explicit_liabilities(streams, accruals);
        assert_eq!(self.actor_balance, liabilities);
        assert!(
            liabilities <= self.total_explicit,
            "conservative explicit reserve is below exact liability"
        );
        let miner = &self.total_minted - &self.total_burn - &self.total_explicit;
        assert!(miner >= TokenAmount::zero());
        assert_eq!(self.f099_balance, &self.total_burn + &self.total_dust);
    }
}
