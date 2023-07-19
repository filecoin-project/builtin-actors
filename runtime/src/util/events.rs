use crate::cbor::serialize_vec;
use crate::ActorError;
use fvm_shared::event::{ActorEvent, Entry, Flags};
use serde::ser;

// Codec identifier for CBOR-encoded data.
const IPLD_CBOR: u64 = 0x51;

/// Builder for ActorEvent objects, accumulating key/value pairs.
pub struct EventBuilder {
    entries: Result<Vec<Entry>, ActorError>,
}

impl EventBuilder {
    /// Creates a new builder with no values.
    pub fn new() -> Self {
        Self { entries: Ok(Vec::new()) }
    }

    /// Pushes an entry with an indexed key and no value.
    pub fn label(mut self, name: &str) -> Self {
        if let Ok(ref mut entries) = self.entries {
            entries.push(Entry {
                flags: Flags::FLAG_INDEXED_KEY,
                key: name.to_string(),
                codec: 0,
                value: vec![],
            });
        }
        self
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
    use crate::util::events::IPLD_CBOR;
    use crate::EventBuilder;
    use fvm_shared::event::{ActorEvent, Entry, Flags};

    #[test]
    fn label() {
        let e = EventBuilder::new().label("l1").label("l2").build().unwrap();
        assert_eq!(
            ActorEvent {
                entries: vec![
                    Entry {
                        flags: Flags::FLAG_INDEXED_KEY,
                        key: "l1".to_string(),
                        codec: 0,
                        value: vec![],
                    },
                    Entry {
                        flags: Flags::FLAG_INDEXED_KEY,
                        key: "l2".to_string(),
                        codec: 0,
                        value: vec![],
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
