use cid::Cid;
use fvm_shared::bigint::BigInt;
use fvm_shared::piece::PaddedPieceSize;
// A namespace for helpers that build and emit provider Actor events.
use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{ActorError, EventBuilder};
use fvm_shared::sector::SectorNumber;

/// Indicates a sector has been pre-committed.
pub fn sector_precommitted(rt: &impl Runtime, sector: SectorNumber) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new().typ("sector-precommitted").field_indexed("sector", &sector).build()?,
    )
}

/// Indicates a sector has been activated.
pub fn sector_activated(
    rt: &impl Runtime,
    sector: SectorNumber,
    unsealed_cid: &Cid,
    pieces: &[(Cid, PaddedPieceSize)],
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("sector-activated")
            .with_sector_info(sector, unsealed_cid, pieces)
            .build()?,
    )
}

/// Indicates a sector has been updated.
pub fn sector_updated(
    rt: &impl Runtime,
    sector: SectorNumber,
    unsealed_cid: &Cid,
    pieces: &[(Cid, PaddedPieceSize)],
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("sector-updated")
            .with_sector_info(sector, unsealed_cid, pieces)
            .build()?,
    )
}

/// Indicates a sector has been terminated.
pub fn sector_terminated(rt: &impl Runtime, sector: SectorNumber) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new().typ("sector-terminated").field_indexed("sector", &sector).build()?,
    )
}

trait WithSectorInfo {
    fn with_sector_info(
        self,
        sector: SectorNumber,
        unsealed_cid: &Cid,
        pieces: &[(Cid, PaddedPieceSize)],
    ) -> EventBuilder;
}

impl WithSectorInfo for EventBuilder {
    fn with_sector_info(
        self,
        sector: SectorNumber,
        unsealed_cid: &Cid,
        pieces: &[(Cid, PaddedPieceSize)],
    ) -> EventBuilder {
        let mut event =
            self.field_indexed("sector", &sector).field_indexed("unsealed-cid", unsealed_cid);

        for piece in pieces {
            event = event
                .field_indexed("piece-cid", &piece.0)
                .field("piece-size", &BigInt::from(piece.1 .0));
        }
        event
    }
}
