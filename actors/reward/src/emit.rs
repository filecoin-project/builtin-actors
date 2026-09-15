use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{ActorError, EventBuilder};
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;

use crate::{PendingWrite, StreamId};

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

pub fn claim_payout(
    rt: &impl Runtime,
    stream_id: StreamId,
    recipient: &Address,
    amount: &TokenAmount,
) -> Result<(), ActorError> {
    let recipient = recipient.id().map_err(|_| {
        fil_actors_runtime::actor_error!(
            illegal_state,
            "claim payout recipient {} is not an ID address",
            recipient
        )
    })?;
    rt.emit_event(
        &EventBuilder::new()
            .typ("claim-payout")
            .field_indexed("stream-id", &stream_id)
            .field_indexed("recipient", &recipient)
            .field("amount", amount)
            .build()?,
    )
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
