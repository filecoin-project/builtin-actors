// A namespace for helpers that build and emit verified registry events.

use crate::{ActorError, AllocationID};
use crate::{ClaimID, DataCap};
use cid::Cid;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::EventBuilder;
use fvm_shared::bigint::bigint_ser::BigIntSer;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::ActorID;

/// Indicates a new value for a verifier's datacap balance.
/// Note that receiving this event does not necessarily mean the balance has changed.
/// The value is in datacap whole units (not TokenAmount).
pub fn verifier_balance(
    rt: &impl Runtime,
    verifier: ActorID,
    prev_balance: &DataCap,
    new_balance: &DataCap,
    client: Option<ActorID>,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("verifier-balance")
            .field_indexed("verifier", &verifier)
            .field_indexed("client", &client)
            .field("prev-balance,", &BigIntSer(prev_balance))
            .field("balance", &BigIntSer(new_balance))
            .build()?,
    )
}

/// Indicates a new allocation has been made.
#[allow(clippy::too_many_arguments)]
pub fn allocation(
    rt: &impl Runtime,
    id: AllocationID,
    client: ActorID,
    provider: ActorID,
    piece_cid: &Cid,
    piece_size: &u64,
    term_min: &ChainEpoch,
    term_max: &ChainEpoch,
    expiration: &ChainEpoch,
) -> Result<(), ActorError> {
    let event = build_base_event(
        "allocation",
        &id,
        &client,
        &provider,
        piece_cid,
        piece_size,
        term_min,
        term_max,
        expiration,
    )
    .build()?;
    rt.emit_event(&event)
}

/// Indicates an expired allocation has been removed.
#[allow(clippy::too_many_arguments)]
pub fn allocation_removed(
    rt: &impl Runtime,
    id: AllocationID,
    client: ActorID,
    provider: ActorID,
    piece_cid: &Cid,
    piece_size: &u64,
    term_min: &ChainEpoch,
    term_max: &ChainEpoch,
    expiration: &ChainEpoch,
) -> Result<(), ActorError> {
    let event = build_base_event(
        "allocation-removed",
        &id,
        &client,
        &provider,
        piece_cid,
        piece_size,
        term_min,
        term_max,
        expiration,
    )
    .build()?;
    rt.emit_event(&event)
}

/// Indicates an allocation has been claimed.
#[allow(clippy::too_many_arguments)]
pub fn claim(
    rt: &impl Runtime,
    id: ClaimID,
    client: ActorID,
    provider: ActorID,
    piece_cid: &Cid,
    piece_size: &u64,
    term_min: &ChainEpoch,
    term_max: &ChainEpoch,
    expiration: &ChainEpoch,
    sector: &u64,
) -> Result<(), ActorError> {
    let event = build_base_event(
        "claim", &id, &client, &provider, piece_cid, piece_size, term_min, term_max, expiration,
    )
    .field_indexed("sector", &sector)
    .build()?;
    rt.emit_event(&event)
}

/// Indicates an existing claim has been updated (e.g. with a longer term).
#[allow(clippy::too_many_arguments)]
pub fn claim_updated(
    rt: &impl Runtime,
    id: ClaimID,
    client: ActorID,
    provider: ActorID,
    piece_cid: &Cid,
    piece_size: &u64,
    term_min: &ChainEpoch,
    term_max: &ChainEpoch,
    expiration: &ChainEpoch,
    sector: &u64,
    prev_term_max: &ChainEpoch,
) -> Result<(), ActorError> {
    let event = build_base_event(
        "claim-updated",
        &id,
        &client,
        &provider,
        piece_cid,
        piece_size,
        term_min,
        term_max,
        expiration,
    )
    .field_indexed("sector", &sector)
    .field("prev-term-max", &prev_term_max)
    .build()?;
    rt.emit_event(&event)
}

/// Indicates an expired claim has been removed.
#[allow(clippy::too_many_arguments)]
pub fn claim_removed(
    rt: &impl Runtime,
    id: ClaimID,
    client: ActorID,
    provider: ActorID,
    piece_cid: &Cid,
    piece_size: &u64,
    term_min: &ChainEpoch,
    term_max: &ChainEpoch,
    expiration: &ChainEpoch,
    sector: &u64,
) -> Result<(), ActorError> {
    let event = build_base_event(
        "claim-removed",
        &id,
        &client,
        &provider,
        piece_cid,
        piece_size,
        term_min,
        term_max,
        expiration,
    )
    .field_indexed("sector", sector)
    .build()?;
    rt.emit_event(&event)
}

#[allow(clippy::too_many_arguments)]
pub fn build_base_event(
    typ: &str,
    id: &u64,
    client: &ActorID,
    provider: &ActorID,
    piece_cid: &Cid,
    piece_size: &u64,
    term_min: &ChainEpoch,
    term_max: &ChainEpoch,
    expiration: &ChainEpoch,
) -> EventBuilder {
    EventBuilder::new()
        .typ(typ)
        .with_parties(id, client, provider)
        .with_piece(piece_cid, piece_size)
        .with_term(term_min, term_max, expiration)
}

// Private helpers //
pub trait WithParties {
    fn with_parties(self, id: &AllocationID, client: &ActorID, provider: &ActorID) -> EventBuilder;
}

impl WithParties for EventBuilder {
    fn with_parties(self, id: &AllocationID, client: &ActorID, provider: &ActorID) -> EventBuilder {
        self.field_indexed("id", &id)
            .field_indexed("client", &client)
            .field_indexed("provider", &provider)
    }
}

pub trait WithPiece {
    fn with_piece(self, piece_cid: &Cid, piece_size: &u64) -> EventBuilder;
}

impl crate::emit::WithPiece for EventBuilder {
    fn with_piece(self, piece_cid: &Cid, piece_size: &u64) -> EventBuilder {
        self.field_indexed("piece-cid", &piece_cid).field("piece-size", &piece_size)
    }
}

pub trait WithTerm {
    fn with_term(
        self,
        term_min: &ChainEpoch,
        term_max: &ChainEpoch,
        expiration: &ChainEpoch,
    ) -> EventBuilder;
}

impl crate::emit::WithTerm for EventBuilder {
    fn with_term(
        self,
        term_min: &ChainEpoch,
        term_max: &ChainEpoch,
        expiration: &ChainEpoch,
    ) -> EventBuilder {
        self.field("term-min", &term_min)
            .field("term-max", &term_max)
            .field("expiration", &expiration)
    }
}
