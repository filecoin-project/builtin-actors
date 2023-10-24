// A namespace for helpers that build and emit verified registry events.

use crate::{ActorError, AllocationID};
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
            .typ("verifier-balance")
            .field_indexed("verifier", &verifier)
            .field("balance", new_balance)
            .build()?,
    )
}

/// Indicates a new allocation has been made.
pub fn allocation(rt: &impl Runtime, id: AllocationID) -> Result<(), ActorError> {
    rt.emit_event(&EventBuilder::new().typ("allocation").field_indexed("id", &id).build()?)
}

/// Indicates an expired allocation has been removed.
pub fn allocation_removed(rt: &impl Runtime, id: AllocationID) -> Result<(), ActorError> {
    rt.emit_event(&EventBuilder::new().typ("allocation-removed").field_indexed("id", &id).build()?)
}

/// Indicates an allocation has been claimed.
pub fn claim(rt: &impl Runtime, id: ClaimID) -> Result<(), ActorError> {
    rt.emit_event(&EventBuilder::new().typ("claim").field_indexed("id", &id).build()?)
}

/// Indicates an existing claim has been updated (e.g. with a longer term).
pub fn claim_updated(rt: &impl Runtime, id: ClaimID) -> Result<(), ActorError> {
    rt.emit_event(&EventBuilder::new().typ("claim-updated").field_indexed("id", &id).build()?)
}

/// Indicates an expired claim has been removed.
pub fn claim_removed(rt: &impl Runtime, id: ClaimID) -> Result<(), ActorError> {
    rt.emit_event(&EventBuilder::new().typ("claim-removed").field_indexed("id", &id).build()?)
}
