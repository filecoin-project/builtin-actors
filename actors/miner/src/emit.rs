// A namespace for helpers that build and emit Miner Actor events.
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{ActorError, EventBuilder};
use fvm_shared::sector::SectorNumber;
use fvm_shared::ActorID;

/// Indicates a sector has been pre-committed.
pub fn sector_precommitted(
    rt: &impl Runtime,
    miner: ActorID,
    sector: SectorNumber,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("sector-precommitted")
            .field_indexed("miner", &miner)
            .field_indexed("sector", &sector)
            .build()?,
    )
}

/// Indicates a sector has been proven.
pub fn sector_activated(
    rt: &impl Runtime,
    miner: ActorID,
    sector: SectorNumber,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("sector-activated")
            .field_indexed("miner", &miner)
            .field_indexed("sector", &sector)
            .build()?,
    )
}
