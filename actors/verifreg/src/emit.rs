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
    new_balance: &DataCap,
    client: Option<ActorID>,
) -> Result<(), ActorError> {
    let mut event: EventBuilder = EventBuilder::new()
        .typ("verifier-balance")
        .field_indexed("verifier", &verifier)
        .field("balance", &BigIntSer(new_balance));
    if let Some(client) = client {
        event = event.field_indexed("client", &client);
    }
    rt.emit_event(&event.build()?)
}

/// Indicates a new allocation has been made.
pub fn allocation(
    rt: &impl Runtime,
    id: AllocationID,
    alloc: &Allocation,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("allocation")
            .with_parties(id, alloc.client, alloc.provider)
            .with_piece(&alloc.data, alloc.size.0)
            .with_term(alloc.term_min, alloc.term_max)
            .field("expiration", &alloc.expiration)
            .build()?,
    )
}

/// Indicates an expired allocation has been removed.
pub fn allocation_removed(
    rt: &impl Runtime,
    id: AllocationID,
    alloc: &Allocation,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("allocation-removed")
            .with_parties(id, alloc.client, alloc.provider)
            .with_piece(&alloc.data, alloc.size.0)
            .with_term(alloc.term_min, alloc.term_max)
            .field("expiration", &alloc.expiration)
            .build()?,
    )
}

/// Indicates an allocation has been claimed.
pub fn claim(rt: &impl Runtime, id: ClaimID, claim: &Claim) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("claim")
            .with_parties(id, claim.client, claim.provider)
            .with_piece(&claim.data, claim.size.0)
            .with_term(claim.term_min, claim.term_max)
            .field("term-start", &claim.term_start)
            .field_indexed("sector", &claim.sector)
            .build()?,
    )
}

/// Indicates an existing claim has been updated (e.g. with a longer term).
pub fn claim_updated(rt: &impl Runtime, id: ClaimID, claim: &Claim) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("claim-updated")
            .with_parties(id, claim.client, claim.provider)
            .with_piece(&claim.data, claim.size.0)
            .with_term(claim.term_min, claim.term_max)
            .field("term-start", &claim.term_start)
            .field_indexed("sector", &claim.sector)
            .build()?,
    )
}

/// Indicates an expired claim has been removed.
pub fn claim_removed(rt: &impl Runtime, id: ClaimID, claim: &Claim) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("claim-removed")
            .with_parties(id, claim.client, claim.provider)
            .with_piece(&claim.data, claim.size.0)
            .with_term(claim.term_min, claim.term_max)
            .field("term-start", &claim.term_start)
            .field_indexed("sector", &claim.sector)
            .build()?,
    )
}

// Private helpers //
trait WithParties {
    fn with_parties(self, id: AllocationID, client: ActorID, provider: ActorID) -> EventBuilder;
}

impl WithParties for EventBuilder {
    fn with_parties(self, id: AllocationID, client: ActorID, provider: ActorID) -> EventBuilder {
        self.field_indexed("id", &id)
            .field_indexed("client", &client)
            .field_indexed("provider", &provider)
    }
}

trait WithPiece {
    fn with_piece(self, piece_cid: &Cid, piece_size: u64) -> EventBuilder;
}

impl crate::emit::WithPiece for EventBuilder {
    fn with_piece(self, piece_cid: &Cid, piece_size: u64) -> EventBuilder {
        self.field_indexed("piece-cid", &piece_cid).field("piece-size", &piece_size)
    }
}

trait WithTerm {
    fn with_term(self, term_min: ChainEpoch, term_max: ChainEpoch) -> EventBuilder;
}

impl crate::emit::WithTerm for EventBuilder {
    fn with_term(self, term_min: ChainEpoch, term_max: ChainEpoch) -> EventBuilder {
        self.field("term-min", &term_min).field("term-max", &term_max)
    }
}
