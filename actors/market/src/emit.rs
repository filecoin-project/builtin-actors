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
            .with_parties(deal_id, client, provider)
            .build()?,
    )
}

/// Indicates a deal has been activated.
pub fn deal_activated(
    rt: &impl Runtime,
    deal_id: DealID,
    client: ActorID,
    provider: ActorID,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("deal-activated")
            .with_parties(deal_id, client, provider)
            .build()?,
    )
}

/// Indicates a deal has been terminated.
pub fn deal_terminated(
    rt: &impl Runtime,
    deal_id: DealID,
    client: ActorID,
    provider: ActorID,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("deal-terminated")
            .with_parties(deal_id, client, provider)
            .build()?,
    )
}

/// Indicates a deal has been completed successfully.
pub fn deal_completed(
    rt: &impl Runtime,
    deal_id: DealID,
    client: ActorID,
    provider: ActorID,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("deal-completed")
            .with_parties(deal_id, client, provider)
            .build()?,
    )
}

trait WithParties {
    fn with_parties(self, id: DealID, client: ActorID, provider: ActorID) -> EventBuilder;
}

impl WithParties for EventBuilder {
    fn with_parties(self, id: DealID, client: ActorID, provider: ActorID) -> EventBuilder {
        self.field_indexed("id", &id)
            .field_indexed("client", &client)
            .field_indexed("provider", &provider)
    }
}
