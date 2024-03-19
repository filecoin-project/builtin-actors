// A namespace for helpers that build and emit verified registry events.

use crate::{ActorError, AllocationID};
use crate::{ClaimID, DataCap};
use cid::Cid;
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::EventBuilder;
use fvm_shared::bigint::bigint_ser::BigIntSer;
use fvm_shared::clock::ChainEpoch;
use fvm_shared::ActorID;

/// Indicates the addition of a new verifier.
/// The value is in datacap whole units (not TokenAmount).
pub fn add_verifier(
    rt: &impl Runtime,
    verifier: ActorID,
    balance: &DataCap,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("add-verifier")
            .field_indexed("verifier", &verifier)
            .field("balance", &BigIntSer(balance))
            .build()?,
    )
}

/// Indicates the removal of a verifier.
pub fn remove_verifier(rt: &impl Runtime, verifier: ActorID) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new().typ("remove-verifier").field_indexed("verifier", &verifier).build()?,
    )
}

/// Indicates the transfer of datacap from verifier to a client.
/// The value is in datacap whole units (not TokenAmount).
pub fn allocate_datacap(
    rt: &impl Runtime,
    verifier: ActorID,
    client: ActorID,
    amount: &DataCap,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("allocate-datacap")
            .field_indexed("verifier", &verifier)
            .field_indexed("client", &client)
            .field("amount", &BigIntSer(amount))
            .build()?,
    )
}

/// Indicates the removal of datacap from a client.
/// The value is in datacap whole units (not TokenAmount).
pub fn remove_datacap(
    rt: &impl Runtime,
    verifier1: ActorID,
    verifier2: ActorID,
    client: ActorID,
    amount: &DataCap,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("remove-datacap")
            .field_indexed("verifier", &verifier1)
            .field_indexed("verifier", &verifier2)
            .field_indexed("client", &client)
            .field("amount", &BigIntSer(amount))
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
