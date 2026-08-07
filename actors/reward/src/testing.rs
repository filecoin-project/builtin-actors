use std::collections::BTreeSet;

use crate::{
    STORAGE_MINING_ALLOCATION, State, StreamsState,
    streams::{compute_service_liability, validate_streams_state},
};
use fil_actors_runtime::MessageAccumulator;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::CborStore;
use fvm_shared::{clock::ChainEpoch, econ::TokenAmount};
use num_traits::Signed;

#[derive(Default)]
pub struct StateSummary {
    pub stream_count: usize,
    pub tombstone_count: usize,
    pub pending_write_count: usize,
}

pub fn check_state_invariants<BS: Blockstore>(
    state: &State,
    store: &BS,
    prior_epoch: ChainEpoch,
    current_epoch: ChainEpoch,
    balance: &TokenAmount,
) -> (StateSummary, MessageAccumulator) {
    let acc = MessageAccumulator::default();

    // Can't assert equality because anyone can send funds to reward actor (and already have on mainnet).
    acc.require(
        (&state.total_minted_reward + balance).atto() >= STORAGE_MINING_ALLOCATION.atto(),
        format!(
            "reward minted {} + reward left {} < storage mining allocation {}",
            state.total_minted_reward, balance, *STORAGE_MINING_ALLOCATION
        ),
    );

    acc.require(
        state.epoch == prior_epoch + 1,
        format!(
            "reward state epoch {} does not match prior_epoch+1 {}",
            state.epoch,
            prior_epoch + 1
        ),
    );
    acc.require(
        state.effective_network_time <= state.epoch,
        format!(
            "effective network time {} greater than state epoch {}",
            state.effective_network_time, state.epoch
        ),
    );
    acc.require(
        state.cumsum_realized <= state.cumsum_baseline,
        format!(
            "cumsum realized {} > cumsum baseline {}",
            state.cumsum_realized, state.cumsum_baseline
        ),
    );
    acc.require(
        !state.cumsum_realized.is_negative(),
        format!("cumsum realized negative ({})", state.cumsum_realized),
    );

    for (name, amount) in [
        ("total minted reward", &state.total_minted_reward),
        ("total burn minted", &state.total_burn_minted),
        ("total explicit minted", &state.total_explicit_minted),
    ] {
        acc.require(!amount.is_negative(), format!("{name} is negative ({amount})"));
    }
    acc.require(
        &state.total_burn_minted + &state.total_explicit_minted <= state.total_minted_reward,
        format!(
            "burn {} + service {} exceeds total minted {}",
            state.total_burn_minted, state.total_explicit_minted, state.total_minted_reward
        ),
    );
    acc.require(
        state.swa_timelock_epochs >= 0,
        format!("SWA timelock is negative ({})", state.swa_timelock_epochs),
    );
    acc.require(
        state.accrued.windows(2).all(|rows| rows[0].id < rows[1].id),
        "explicit-stream accrual rows are not strictly ordered by stream ID",
    );
    for row in &state.accrued {
        acc.require(
            !row.amount.is_negative(),
            format!("explicit-stream accrual for stream {} is negative ({})", row.id, row.amount),
        );
    }

    let streams_state = match store.get_cbor::<StreamsState>(&state.streams_root) {
        Ok(Some(streams_state)) => streams_state,
        Ok(None) => {
            acc.add("error loading streams state: root not found");
            return (StateSummary::default(), acc);
        }
        Err(e) => {
            acc.add(format!("error loading streams state: {e}"));
            return (StateSummary::default(), acc);
        }
    };
    if let Err(error) = validate_streams_state(&streams_state, &state.accrued, current_epoch) {
        acc.add(format!("invalid streams state: {error}"));
    }
    let summary = StateSummary {
        stream_count: streams_state.streams.len(),
        tombstone_count: streams_state.tombstones.len(),
        pending_write_count: streams_state.pending_writes.len(),
    };

    acc.require(
        streams_state.streams.windows(2).all(|rows| rows[0].id < rows[1].id),
        "streams are not strictly ordered by stream ID",
    );
    acc.require(
        streams_state.tombstones.windows(2).all(|rows| rows[0].id < rows[1].id),
        "tombstones are not strictly ordered by stream ID",
    );

    let stream_ids: BTreeSet<_> = streams_state.streams.iter().map(|stream| stream.id).collect();
    let explicit_stream_ids: BTreeSet<_> = streams_state
        .streams
        .iter()
        .filter(|stream| stream.distribution.is_some())
        .map(|stream| stream.id)
        .collect();
    let accrual_ids: BTreeSet<_> = state.accrued.iter().map(|row| row.id).collect();
    let tombstone_ids: BTreeSet<_> =
        streams_state.tombstones.iter().map(|tombstone| tombstone.id).collect();
    acc.require(stream_ids.is_disjoint(&tombstone_ids), "a stream ID is both live and tombstoned");

    let missing_accruals: Vec<_> = explicit_stream_ids.difference(&accrual_ids).copied().collect();
    let unexpected_accruals: Vec<_> =
        accrual_ids.difference(&explicit_stream_ids).copied().collect();
    acc.require(
        missing_accruals.is_empty() && unexpected_accruals.is_empty(),
        format!(
            "explicit-stream accrual IDs do not match live explicit streams: missing {missing_accruals:?}, unexpected {unexpected_accruals:?}"
        ),
    );

    let mut pending_slots = BTreeSet::new();
    for write in &streams_state.pending_writes {
        acc.require(
            pending_slots.insert((write.id, write.op)),
            format!("duplicate pending slot ({:?}, {:?})", write.id, write.op),
        );
    }
    acc.require(
        streams_state
            .pending_writes
            .windows(2)
            .all(|writes| writes[0].effective_epoch <= writes[1].effective_epoch),
        "pending writes are not ordered by effective epoch",
    );

    match compute_service_liability(&streams_state, &state.accrued) {
        Ok(liabilities) => {
            acc.require(
                liabilities <= state.total_explicit_minted,
                format!(
                    "explicit-stream liabilities {liabilities} exceed total explicit minted {}",
                    state.total_explicit_minted
                ),
            );
            acc.require(
                balance >= &liabilities,
                format!(
                    "reward balance {balance} does not cover explicit-stream liabilities {liabilities}"
                ),
            );
        }
        Err(error) => acc.add(format!("error computing explicit-stream liabilities: {error}")),
    }

    (summary, acc)
}
