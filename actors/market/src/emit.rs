use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{ActorError, EventBuilder};
use fvm_shared::deal::DealID;
use fvm_shared::ActorID;

/// Indicates a deal has been published.
pub fn deal_published(
    rt: &impl Runtime,
    client: ActorID,
    provider: ActorID,
    deal_id: DealID,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("deal-published")
            .field_indexed("client", &client)
            .field_indexed("provider", &provider)
            .field_indexed("deal_id", &deal_id)
            .build()?,
    )
}

/// Indicates a deal has been activated.
pub fn deal_activated(rt: &impl Runtime, deal_id: DealID) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new().typ("deal-activated").field_indexed("deal_id", &deal_id).build()?,
    )
}

/// Indicates a deal has been terminated.
pub fn deal_terminated(rt: &impl Runtime, deal_id: DealID) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new().typ("deal-terminated").field_indexed("deal_id", &deal_id).build()?,
    )
}
