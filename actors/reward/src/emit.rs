use fil_actors_runtime::runtime::Runtime;
use fil_actors_runtime::{ActorError, EventBuilder, actor_error};
use fvm_ipld_encoding::tuple::*;
use fvm_shared::ActorID;
use fvm_shared::address::Address;
use fvm_shared::econ::TokenAmount;

use crate::{Fold, FoldCause, MAX_RECIPIENTS, PendingWrite, SharesInstalled, StreamId};

/// FVM rejects an event whose values exceed this in total.
const EVENT_VALUES_BUDGET: usize = 8 << 10;

/// Widest CBOR for one `(recipient, share)` row, a 2-element array header and two full-width u64s.
const MAX_SHARE_ROW_BYTES: usize = 1 + 9 + 9;

/// Widest CBOR for `$type` and `stream-id`, with padding.
const MAX_SCALAR_FIELD_BYTES: usize = 64;

/// Widest CBOR for `shares-set`, which grows with state.
const MAX_SHARES_SET_BYTES: usize =
    3 + MAX_RECIPIENTS * MAX_SHARE_ROW_BYTES + MAX_SCALAR_FIELD_BYTES;

const _: () = assert!(
    MAX_SHARES_SET_BYTES < EVENT_VALUES_BUDGET,
    "a full share map exceeds the FVM event value budget"
);

/// One row of a `shares-set` map.
#[derive(Serialize_tuple)]
struct ShareRow {
    recipient: ActorID,
    share: u64,
}

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

/// Announces a closed period: the pool it divided and the remainder left to burn.
pub fn period_folded(rt: &impl Runtime, fold: &Fold) -> Result<(), ActorError> {
    rt.emit_event(&period_folded_event(fold).build()?)
}

/// Announces the share map a stream now pays by.
pub fn shares_set(rt: &impl Runtime, installed: &SharesInstalled) -> Result<(), ActorError> {
    rt.emit_event(&shares_set_event(installed)?.build()?)
}

/// Announces one recipient row moving to a new address, or to f099 to drop the share.
pub fn address_replaced(
    rt: &impl Runtime,
    stream_id: StreamId,
    old: &Address,
    new: &Address,
) -> Result<(), ActorError> {
    rt.emit_event(&address_replaced_event(stream_id, old, new)?.build()?)
}

pub fn claim_payout(
    rt: &impl Runtime,
    stream_id: StreamId,
    recipient: &Address,
    amount: &TokenAmount,
) -> Result<(), ActorError> {
    rt.emit_event(
        &EventBuilder::new()
            .typ("claim-payout")
            .field_indexed("stream-id", &stream_id)
            .field_indexed("recipient", &actor_id(recipient, "claim payout recipient")?)
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

fn period_folded_event(fold: &Fold) -> EventBuilder {
    EventBuilder::new()
        .typ("period-folded")
        .field_indexed("stream-id", &fold.id)
        .field("cause", FoldCause::as_str(fold.cause))
        .field("accrued", &fold.accrued)
        .field("dust", &fold.dust)
}

fn shares_set_event(installed: &SharesInstalled) -> Result<EventBuilder, ActorError> {
    let rows = installed
        .shares
        .iter()
        .map(|row| {
            Ok(ShareRow {
                recipient: actor_id(&row.recipient, "share recipient")?,
                share: row.share,
            })
        })
        .collect::<Result<Vec<ShareRow>, ActorError>>()?;
    Ok(EventBuilder::new()
        .typ("shares-set")
        .field_indexed("stream-id", &installed.id)
        .field("shares", &rows))
}

fn address_replaced_event(
    stream_id: StreamId,
    old: &Address,
    new: &Address,
) -> Result<EventBuilder, ActorError> {
    Ok(EventBuilder::new()
        .typ("address-replaced")
        .field_indexed("stream-id", &stream_id)
        .field_indexed("old-recipient", &actor_id(old, "old recipient")?)
        .field_indexed("new-recipient", &actor_id(new, "new recipient")?))
}

/// Stored recipients are ID addresses, so anything else is a state fault, not a bad call.
fn actor_id(address: &Address, label: &str) -> Result<ActorID, ActorError> {
    address
        .id()
        .map_err(|_| actor_error!(illegal_state, "{} {} is not an ID address", label, address))
}

#[cfg(test)]
mod tests {
    use fvm_ipld_encoding::RawBytes;
    use fvm_shared::clock::ChainEpoch;
    use fvm_shared::event::ActorEvent;

    use super::*;
    use crate::state::{RecipientShare, WeightRecord};
    use crate::types::{DistributionInit, RegisterStreamPayload};

    /// FVM's key limit, from the same syscall as [`EVENT_VALUES_BUDGET`].
    const MAX_KEY_LEN: usize = 31;

    fn widest_shares() -> Vec<RecipientShare> {
        (0..MAX_RECIPIENTS)
            .map(|row| RecipientShare {
                recipient: Address::new_id(ActorID::MAX - row as u64),
                share: u64::MAX,
            })
            .collect()
    }

    fn values_len(event: &ActorEvent) -> usize {
        event.entries.iter().map(|entry| entry.value.len()).sum()
    }

    #[test]
    fn a_full_share_map_fits_the_event_budget() {
        let installed = SharesInstalled { id: StreamId::MAX, shares: widest_shares() };
        let event = shares_set_event(&installed).unwrap().build().unwrap();
        let values = values_len(&event);
        assert!(values <= MAX_SHARES_SET_BYTES, "{values} bytes exceeds the derived ceiling");
        assert!(values < EVENT_VALUES_BUDGET, "{values} bytes exceeds the FVM budget");
    }

    /// A registration's initial map, in address form, is the widest payload `write-queued`
    /// ever emits.
    #[test]
    fn the_widest_queued_payload_fits_the_event_budget() {
        let payload = RawBytes::serialize(&RegisterStreamPayload {
            weight: WeightRecord {
                v_start: u64::MAX,
                slope: i64::MIN,
                t_start: ChainEpoch::MAX,
                floor: u64::MAX,
                cap: u64::MAX,
            },
            distribution: Some(DistributionInit {
                writer: Address::new_id(ActorID::MAX),
                shares: widest_shares(),
            }),
        })
        .unwrap();
        let write = PendingWrite {
            id: Some(StreamId::MAX),
            op: crate::state::PendingWriteOp::RegisterStream,
            payload,
            effective_epoch: ChainEpoch::MAX,
        };
        let values = values_len(
            &write_event("write-queued", &write).field("payload", &write.payload).build().unwrap(),
        );
        assert!(values < EVENT_VALUES_BUDGET, "{values} bytes exceeds the FVM budget");
    }

    #[test]
    fn event_keys_stay_within_the_key_limit() {
        let installed = SharesInstalled { id: 1, shares: widest_shares() };
        let fold = Fold {
            id: 1,
            cause: FoldCause::SetShares,
            accrued: TokenAmount::from_atto(1),
            dust: TokenAmount::from_atto(1),
        };
        let events = [
            shares_set_event(&installed).unwrap().build().unwrap(),
            period_folded_event(&fold).build().unwrap(),
            address_replaced_event(1, &Address::new_id(2), &Address::new_id(3))
                .unwrap()
                .build()
                .unwrap(),
        ];
        for event in events {
            for entry in &event.entries {
                assert!(entry.key.len() <= MAX_KEY_LEN, "key {} is too long", entry.key);
            }
        }
    }
}
