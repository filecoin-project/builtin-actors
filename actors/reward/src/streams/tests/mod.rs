use fvm_shared::address::Address;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::econ::TokenAmount;
use num_traits::Zero;

use super::*;

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

/// The cancellation path as the actor drives it: resolve the slot, then empty it.
fn cancel(
    streams: &mut StreamsState,
    id: Option<StreamId>,
    op: PendingWriteOp,
) -> anyhow::Result<Option<PendingWrite>> {
    Ok(super::queue::cancel_pending(streams, Slot::for_cancel(id, op)?))
}

fn amount(rows: &RecipientTable, recipient: u64) -> TokenAmount {
    rows.get(&Address::new_id(recipient))
}

fn explicit_liabilities(streams: &StreamsState, accruals: &[StreamAccrual]) -> TokenAmount {
    explicit_liability(streams, accruals).unwrap()
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
        let allocation = allocate_reward(&streams.streams, epoch, &reward).unwrap();
        assert!(allocation.schedule_valid, "valid randomized state entered degradation");
        let explicit =
            allocation.portions.iter().fold(TokenAmount::zero(), |total, row| total + &row.amount);
        accrue_explicit(accruals, &allocation.portions);
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
