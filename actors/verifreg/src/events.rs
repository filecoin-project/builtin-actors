use crate::{Allocation, AllocationID, Claim, ClaimID, DataCap};
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{ActorError, EventBuilder};
use fvm_shared::ActorID;

// A namespace for helpers that build and emit verified registry events.
// REVIEW: would this be better as a simple module with pub fns, called as emit::verifier_balance()?
pub struct Emit {}

impl Emit {
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
                .label("verifier-balance")
                .value_indexed("verifier", &verifier)?
                .value("balance", new_balance)?
                .build(),
        )
    }

    /// Indicates a new allocation has been made.
    pub fn allocation(
        rt: &impl Runtime,
        id: AllocationID,
        alloc: &Allocation,
    ) -> Result<(), ActorError> {
        rt.emit_event(&EventBuilder::new().label("allocation").with_allocation(id, alloc)?.build())
    }

    /// Indicates an expired allocation has been removed.
    pub fn allocation_removed(
        rt: &impl Runtime,
        id: AllocationID,
        alloc: &Allocation,
    ) -> Result<(), ActorError> {
        rt.emit_event(
            &EventBuilder::new().label("allocation-removed").with_allocation(id, alloc)?.build(),
        )
    }

    /// Indicates an allocation has been claimed.
    pub fn claim(rt: &impl Runtime, id: ClaimID, claim: &Claim) -> Result<(), ActorError> {
        rt.emit_event(&EventBuilder::new().label("claim").with_claim(id, claim)?.build())
    }

    /// Indicates an existing claim has been updated (e.g. with a longer term).
    pub fn claim_updated(rt: &impl Runtime, id: ClaimID, claim: &Claim) -> Result<(), ActorError> {
        rt.emit_event(&EventBuilder::new().label("claim-updated").with_claim(id, claim)?.build())
    }

    /// Indicates an expired claim has been removed.
    pub fn claim_removed(rt: &impl Runtime, id: ClaimID, claim: &Claim) -> Result<(), ActorError> {
        rt.emit_event(&EventBuilder::new().label("claim-removed").with_claim(id, claim)?.build())
    }
}

trait WithAllocation {
    fn with_allocation(
        self,
        id: AllocationID,
        alloc: &Allocation,
    ) -> Result<EventBuilder, ActorError>;
}

impl WithAllocation for EventBuilder {
    fn with_allocation(
        self,
        id: AllocationID,
        alloc: &Allocation,
    ) -> Result<EventBuilder, ActorError> {
        self.value_indexed("id", &id)?
            .value_indexed("client", &alloc.client)?
            .value_indexed("provider", &alloc.provider)?
            .value_indexed("data-cid", &alloc.data)?
            .value("data-size", &alloc.size)?
            .value("term-min", &alloc.term_min)?
            .value("term-max", &alloc.term_max)?
            .value("expiration", &alloc.expiration)
    }
}

trait WithClaim {
    fn with_claim(self, id: ClaimID, claim: &Claim) -> Result<EventBuilder, ActorError>;
}

impl WithClaim for EventBuilder {
    fn with_claim(self, id: ClaimID, claim: &Claim) -> Result<EventBuilder, ActorError> {
        self.value_indexed("id", &id)?
            .value_indexed("provider", &claim.provider)?
            .value_indexed("client", &claim.client)?
            .value_indexed("data-cid", &claim.data)?
            .value("data-size", &claim.size)?
            .value("term-min", &claim.term_min)?
            .value("term-max", &claim.term_max)?
            .value("term-start", &claim.term_start)?
            .value("sector", &claim.sector)
    }
}
