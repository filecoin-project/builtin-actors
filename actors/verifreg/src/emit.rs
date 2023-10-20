// A namespace for helpers that build and emit verified registry events.

use crate::{ActorError, Allocation, AllocationID, Claim};
use crate::{ClaimID, DataCap};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::EventBuilder;
use fvm_shared::ActorID;

/// Indicates a new value for a verifier's datacap balance.
/// Note that receiving this event does not necessarily mean the balance has changed.
/// The value is in datacap whole units (not TokenAmount).
pub fn verifier_balance(
    rt: &impl Runtime,
    verifier: ActorID,
    new_balance: &DataCap,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .event_type("verifier-balance")
            .field_indexed("verifier", &verifier)
            .field("balance", new_balance)
            .build()?,
    )
}

/// Indicates a new allocation has been made.
pub fn allocation(
    rt: &impl Runtime,
    id: AllocationID,
    alloc: &Allocation,
) -> Result<(), ActorError> {
    rt.emit_event(&EventBuilder::new().event_type("allocation").with_allocation(id, alloc).build()?)
}

/// Indicates an expired allocation has been removed.
pub fn allocation_removed(
    rt: &impl Runtime,
    id: AllocationID,
    alloc: &Allocation,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new().event_type("allocation-removed").with_allocation(id, alloc).build()?,
    )
}

/// Indicates an allocation has been claimed.
pub fn claim(rt: &impl Runtime, id: ClaimID, claim: &Claim) -> Result<(), ActorError> {
    rt.emit_event(&EventBuilder::new().event_type("claim").with_claim(id, claim).build()?)
}

// Private helpers //
trait WithAllocation {
    fn with_allocation(self, id: AllocationID, alloc: &Allocation) -> EventBuilder;
}

impl WithAllocation for EventBuilder {
    fn with_allocation(self, id: AllocationID, alloc: &Allocation) -> EventBuilder {
        self.field_indexed("id", &id)
            .field_indexed("client", &alloc.client)
            .field_indexed("provider", &alloc.provider)
            .field_indexed("data-cid", &alloc.data)
    }
}

trait WithClaim {
    fn with_claim(self, id: ClaimID, claim: &Claim) -> EventBuilder;
}

impl WithClaim for EventBuilder {
    fn with_claim(self, id: ClaimID, claim: &Claim) -> EventBuilder {
        self.field_indexed("id", &id)
            .field_indexed("provider", &claim.provider)
            .field_indexed("client", &claim.client)
            .field_indexed("data-cid", &claim.data)
    }
}