use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{ActorError, EventBuilder};

use crate::PendingWrite;

pub fn write_queued(rt: &impl Runtime, write: &PendingWrite) -> Result<(), ActorError> {
    rt.emit_event(&write_event("write-queued", write).field("payload", &write.payload).build()?)
}

pub fn write_cancelled(rt: &impl Runtime, write: &PendingWrite) -> Result<(), ActorError> {
    rt.emit_event(&write_event("write-cancelled", write).build()?)
}

pub fn write_applied(rt: &impl Runtime, write: &PendingWrite) -> Result<(), ActorError> {
    rt.emit_event(&write_event("write-applied", write).build()?)
}

pub fn write_dropped(rt: &impl Runtime, write: &PendingWrite) -> Result<(), ActorError> {
    rt.emit_event(&write_event("write-dropped", write).build()?)
}

fn write_event(typ: &'static str, write: &PendingWrite) -> EventBuilder {
    let event = EventBuilder::new()
        .typ(typ)
        .field_indexed("op", &write.op)
        .field("effective-epoch", &write.effective_epoch);
    match write.id {
        Some(id) => event.field_indexed("stream-id", &id),
        None => event,
    }
}
