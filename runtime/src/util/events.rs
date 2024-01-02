use crate::cbor::serialize_vec;
use crate::ActorError;
use fvm_shared::event::{ActorEvent, Entry, Flags};
use serde::ser;

// Codec identifier for CBOR-encoded data.
const IPLD_CBOR: u64 = 0x51;

const EVENT_TYPE_KEY: &str = "$type";

/// Builder for ActorEvent objects, accumulating key/value pairs.
pub struct EventBuilder {
    entries: Result<Vec<Entry>, ActorError>,
}

impl EventBuilder {
    /// Creates a new builder with no values.
    pub fn new() -> Self {
        Self { entries: Ok(Vec::new()) }
    }

    /// Initialise the "type" of the event i.e. Actor event type.
    pub fn typ(self, _type: &str) -> Self {
        self.push_entry(EVENT_TYPE_KEY, _type, Flags::FLAG_INDEXED_ALL)
    }

    /// Pushes an entry with an indexed key and an un-indexed, IPLD-CBOR-serialized value.
    pub fn field<T: ser::Serialize + ?Sized>(self, name: &str, value: &T) -> Self {
        self.push_entry(name, value, Flags::FLAG_INDEXED_KEY)
    }

    /// Pushes an entry with an indexed key and indexed, IPLD-CBOR-serialized value.
    pub fn field_indexed<T: ser::Serialize + ?Sized>(self, name: &str, value: &T) -> Self {
        self.push_entry(name, value, Flags::FLAG_INDEXED_ALL)
    }

    /// Returns an actor event ready to emit (consuming self).
    pub fn build(self) -> Result<ActorEvent, ActorError> {
        Ok(ActorEvent { entries: self.entries? })
    }

    /// Pushes an entry with an IPLD-CBOR-serialized value.
    fn push_entry<T: ser::Serialize + ?Sized>(
        mut self,
        key: &str,
        value: &T,
        flags: Flags,
    ) -> Self {
        if let Ok(ref mut entries) = self.entries {
            match serialize_vec(&value, "event value") {
                Ok(value) => {
                    entries.push(Entry { flags, key: key.to_string(), codec: IPLD_CBOR, value })
                }
                Err(e) => {
                    self.entries = Err(e);
                }
            }
        }
        self
    }
}

impl Default for EventBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod test {
    use crate::cbor::serialize_vec;
    use crate::util::events::{EVENT_TYPE_KEY, IPLD_CBOR};
    use crate::EventBuilder;
    use fvm_shared::event::{ActorEvent, Entry, Flags};

    #[test]
    fn event_type() {
        let e = EventBuilder::new().typ("l1").field_indexed("v1", "abc").build().unwrap();

        let l1_cbor = serialize_vec("l1", "event value").unwrap();
        let v_cbor = serialize_vec("abc", "event value").unwrap();

        assert_eq!(
            ActorEvent {
                entries: vec![
                    Entry {
                        flags: Flags::FLAG_INDEXED_ALL,
                        key: EVENT_TYPE_KEY.to_string(),
                        codec: IPLD_CBOR,
                        value: l1_cbor, // CBOR for "l1"
                    },
                    Entry {
                        flags: Flags::FLAG_INDEXED_ALL,
                        key: "v1".to_string(),
                        codec: IPLD_CBOR,
                        value: v_cbor, // CBOR for "abc"
                    },
                ]
            },
            e
        )
    }

    #[test]
    fn values() {
        let e = EventBuilder::new().field("v1", &3).field_indexed("v2", "abc").build().unwrap();
        assert_eq!(
            ActorEvent {
                entries: vec![
                    Entry {
                        flags: Flags::FLAG_INDEXED_KEY,
                        key: "v1".to_string(),
                        codec: IPLD_CBOR,
                        value: vec![0x03],
                    },
                    Entry {
                        flags: Flags::FLAG_INDEXED_ALL,
                        key: "v2".to_string(),
                        codec: IPLD_CBOR,
                        value: vec![0x63, 0x61, 0x62, 0x63], // CBOR for "abc"
                    },
                ]
            },
            e
        );
    }
}
