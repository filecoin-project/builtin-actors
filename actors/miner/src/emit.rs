// A namespace for helpers that build and emit Miner Actor events.

use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{ActorError, EventBuilder};
use fvm_shared::sector::SectorID;

/// Indicates a sector has been pre-committed.
pub fn sector_precommited(rt: &impl Runtime, id: SectorID) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("sector-precommited")
            .field_indexed("miner", &id.miner)
            .field_indexed("sector", &id.number)
            .build()?,
    )
}
