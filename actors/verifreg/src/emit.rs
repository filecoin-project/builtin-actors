// A namespace for helpers that build and emit verified registry events.

use crate::{ActorError, Allocation, AllocationID, Claim};
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
    client: Option<ActorID>,
    new_balance: &DataCap,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("verifier-balance")
            .field_indexed("verifier", &verifier)
            .field_indexed("client", &client)
            .field("balance", &BigIntSer(new_balance))
            .build()?,
    )
}

/// Indicates a new allocation has been made.
pub fn allocation(
    rt: &impl Runtime,
    id: AllocationID,
    alloc: &Allocation,
) -> Result<(), ActorError> {
    let event = build_base_event(
        "allocation",
        id,
        alloc.client,
        alloc.provider,
        &alloc.data,
        alloc.size.0,
        alloc.term_min,
        alloc.term_max,
    )
    .field("expiration", &alloc.expiration)
    .build()?;
    rt.emit_event(&event)
}

/// Indicates an expired allocation has been removed.
pub fn allocation_removed(
    rt: &impl Runtime,
    id: AllocationID,
    alloc: &Allocation,
) -> Result<(), ActorError> {
    let event = build_base_event(
        "allocation-removed",
        id,
        alloc.client,
        alloc.provider,
        &alloc.data,
        alloc.size.0,
        alloc.term_min,
        alloc.term_max,
    )
    .field("expiration", &alloc.expiration)
    .build()?;
    rt.emit_event(&event)
}

/// Indicates an allocation has been claimed.
pub fn claim(rt: &impl Runtime, id: ClaimID, claim: &Claim) -> Result<(), ActorError> {
    let event = build_base_event(
        "claim",
        id,
        claim.client,
        claim.provider,
        &claim.data,
        claim.size.0,
        claim.term_min,
        claim.term_max,
    )
    .field_indexed("sector", &claim.sector)
    .build()?;
    rt.emit_event(&event)
}

/// Indicates an existing claim has been updated (e.g. with a longer term).
pub fn claim_updated(rt: &impl Runtime, id: ClaimID, claim: &Claim) -> Result<(), ActorError> {
    let event = build_base_event(
        "claim-updated",
        id,
        claim.client,
        claim.provider,
        &claim.data,
        claim.size.0,
        claim.term_min,
        claim.term_max,
    )
    .field_indexed("sector", &claim.sector)
    .build()?;
    rt.emit_event(&event)
}

/// Indicates an expired claim has been removed.
pub fn claim_removed(rt: &impl Runtime, id: ClaimID, claim: &Claim) -> Result<(), ActorError> {
    let event = build_base_event(
        "claim-removed",
        id,
        claim.client,
        claim.provider,
        &claim.data,
        claim.size.0,
        claim.term_min,
        claim.term_max,
    )
    .field_indexed("sector", &claim.sector)
    .build()?;
    rt.emit_event(&event)
}

#[allow(clippy::too_many_arguments)]
pub fn build_base_event(
    typ: &str,
    id: u64,
    client: ActorID,
    provider: ActorID,
    piece_cid: &Cid,
    piece_size: u64,
    term_min: ChainEpoch,
    term_max: ChainEpoch,
) -> EventBuilder {
    EventBuilder::new()
        .typ(typ)
        .with_parties(id, client, provider)
        .with_piece(piece_cid, piece_size)
        .with_term(term_min, term_max)
}

// Private helpers //
pub trait WithParties {
    fn with_parties(self, id: AllocationID, client: ActorID, provider: ActorID) -> EventBuilder;
}

impl WithParties for EventBuilder {
    fn with_parties(self, id: AllocationID, client: ActorID, provider: ActorID) -> EventBuilder {
        self.field_indexed("id", &id)
            .field_indexed("client", &client)
            .field_indexed("provider", &provider)
    }
}

pub trait WithPiece {
    fn with_piece(self, piece_cid: &Cid, piece_size: u64) -> EventBuilder;
}

impl crate::emit::WithPiece for EventBuilder {
    fn with_piece(self, piece_cid: &Cid, piece_size: u64) -> EventBuilder {
        self.field_indexed("piece-cid", &piece_cid).field("piece-size", &piece_size)
    }
}

pub trait WithTerm {
    fn with_term(self, term_min: ChainEpoch, term_max: ChainEpoch) -> EventBuilder;
}

impl crate::emit::WithTerm for EventBuilder {
    fn with_term(self, term_min: ChainEpoch, term_max: ChainEpoch) -> EventBuilder {
        self.field("term-min", &term_min).field("term-max", &term_max)
    }
}
