use std::collections::BTreeSet;

use crate::{State, StreamsState, streams::validate_streams_state};
use fil_actors_runtime::MessageAccumulator;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::CborStore;
use fvm_shared::{clock::ChainEpoch, econ::TokenAmount};
use num_traits::{Signed, Zero};

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
    balance: &TokenAmount,
) -> (StateSummary, MessageAccumulator) {
    let acc = MessageAccumulator::default();

    let storage_mining_allocation_check = TokenAmount::from_whole(1_100_000_000);

    // Can't assert equality because anyone can send funds to reward actor (and already have on mainnet).
    acc.require(
        &state.total_minted_reward + balance >= storage_mining_allocation_check,
        format!(
            "reward minted {} + reward left {} < storage mining allocation {}",
            state.total_minted_reward, balance, storage_mining_allocation_check
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
        ("total service minted", &state.total_service_minted),
    ] {
        acc.require(!amount.is_negative(), format!("{name} is negative ({amount})"));
    }
    acc.require(
        &state.total_burn_minted + &state.total_service_minted <= state.total_minted_reward,
        format!(
            "burn {} + service {} exceeds total minted {}",
            state.total_burn_minted, state.total_service_minted, state.total_minted_reward
        ),
    );
    acc.require(
        state.swa_timelock_epochs >= 0,
        format!("SWA timelock is negative ({})", state.swa_timelock_epochs),
    );
    acc.require(
        state.service_accrued.windows(2).all(|rows| rows[0].id < rows[1].id),
        "service accrual rows are not strictly ordered by stream ID",
    );
    for row in &state.service_accrued {
        acc.require(
            !row.amount.is_negative(),
            format!("service accrual for stream {} is negative ({})", row.id, row.amount),
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
    if let Err(error) = validate_streams_state(&streams_state, &state.service_accrued, state.epoch)
    {
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
    let accrual_ids: BTreeSet<_> = state.service_accrued.iter().map(|row| row.id).collect();
    let tombstone_ids: BTreeSet<_> =
        streams_state.tombstones.iter().map(|tombstone| tombstone.id).collect();
    acc.require(stream_ids.is_disjoint(&tombstone_ids), "a stream ID is both live and tombstoned");

    let missing_accruals: Vec<_> = explicit_stream_ids.difference(&accrual_ids).copied().collect();
    let unexpected_accruals: Vec<_> =
        accrual_ids.difference(&explicit_stream_ids).copied().collect();
    acc.require(
        missing_accruals.is_empty() && unexpected_accruals.is_empty(),
        format!(
            "service accrual IDs do not match live explicit streams: missing {missing_accruals:?}, unexpected {unexpected_accruals:?}"
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

    let expected_next_transition = streams_state
        .pending_writes
        .first()
        .map_or(fvm_shared::clock::EPOCH_UNDEFINED, |write| write.effective_epoch);
    acc.require(
        state.next_transition_epoch == expected_next_transition,
        format!(
            "next transition epoch {} does not match queue head {}",
            state.next_transition_epoch, expected_next_transition
        ),
    );

    let mut liabilities = TokenAmount::zero();
    for row in &state.service_accrued {
        liabilities += &row.amount;
    }
    for stream in &streams_state.streams {
        if let Some(distribution) = &stream.distribution {
            for row in &distribution.payable {
                acc.require(
                    !row.amount.is_negative(),
                    format!("payable for stream {} is negative ({})", stream.id, row.amount),
                );
                liabilities += &row.amount;
            }
            for row in &distribution.claimed_period {
                acc.require(
                    !row.amount.is_negative(),
                    format!("claimed amount for stream {} is negative ({})", stream.id, row.amount),
                );
                liabilities -= &row.amount;
            }
        }
    }
    for tombstone in &streams_state.tombstones {
        for row in &tombstone.payable {
            acc.require(
                !row.amount.is_negative(),
                format!(
                    "tombstone payable for stream {} is negative ({})",
                    tombstone.id, row.amount
                ),
            );
            liabilities += &row.amount;
        }
    }
    acc.require(
        !liabilities.is_negative(),
        format!("service liabilities are negative ({liabilities})"),
    );
    acc.require(
        liabilities <= state.total_service_minted,
        format!(
            "service liabilities {liabilities} exceed total service minted {}",
            state.total_service_minted
        ),
    );
    acc.require(
        balance >= &liabilities,
        format!("reward balance {balance} does not cover service liabilities {liabilities}"),
    );

    (summary, acc)
}
