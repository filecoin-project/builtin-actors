use std::collections::BTreeSet;

use crate::{STORAGE_MINING_ALLOCATION, State, StreamsState, streams::validate_streams_state};
use fil_actors_runtime::MessageAccumulator;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_encoding::CborStore;
use fvm_shared::{address::Protocol, clock::ChainEpoch, econ::TokenAmount};
use num_traits::Signed;

#[derive(Default)]
pub struct StateSummary {
    pub stream_count: usize,
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
            "burn {} + explicit {} exceeds total minted {}",
            state.total_burn_minted, state.total_explicit_minted, state.total_minted_reward
        ),
    );
    acc.require(
        state.swa_timelock_epochs >= 0,
        format!("SWA timelock is negative ({})", state.swa_timelock_epochs),
    );
    acc.require(
        state.swa_actor.protocol() == Protocol::ID,
        format!("SWA actor {} is not an ID address", state.swa_actor),
    );

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
    if let Err(error) = validate_streams_state(&streams_state, current_epoch) {
        acc.add(format!("invalid streams state: {error}"));
    }
    let summary = StateSummary {
        stream_count: streams_state.streams.len(),
        pending_write_count: streams_state.pending_writes.len(),
    };

    acc.require(
        streams_state.streams.windows(2).all(|rows| rows[0].id < rows[1].id),
        "streams are not strictly ordered by stream ID",
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

    (summary, acc)
}
