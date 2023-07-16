use crate::cbor::serialize_vec;
use crate::ActorError;
use fvm_shared::event::{ActorEvent, Entry, Flags};
use serde::ser;

// Codec identifier for CBOR-encoded data.
const IPLD_CBOR: u64 = 0x51;

/// Builder for ActorEvent objects, accumulating key/value pairs.
pub struct EventBuilder {
    entries: Vec<Entry>,
}

impl EventBuilder {
    /// Creates a new builder with no values.
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Pushes an entry with an indexed key and no value.
    pub fn label(mut self, name: &str) -> Self {
        self.entries.push(Entry {
            flags: Flags::FLAG_INDEXED_KEY,
            key: name.to_string(),
            codec: 0,
            value: vec![],
        });
        self
    }

    /// Pushes an entry with an indexed key and an un-indexed, IPLD-CBOR-serialized value.
    pub fn value<T: ser::Serialize + ?Sized>(
        mut self,
        name: &str,
        value: &T,
    ) -> Result<Self, ActorError> {
        self.entries.push(Entry {
            flags: Flags::FLAG_INDEXED_KEY,
            key: name.to_string(),
            codec: IPLD_CBOR,
            value: serialize_vec(&value, "event value")?,
        });
        Ok(self)
    }

    /// Pushes an entry with an indexed key and indexed, IPLD-CBOR-serialized value.
    pub fn value_indexed<T: ser::Serialize + ?Sized>(
        mut self,
        name: &str,
        value: &T,
    ) -> Result<Self, ActorError> {
        self.entries.push(Entry {
            flags: Flags::FLAG_INDEXED_ALL,
            key: name.to_string(),
            codec: IPLD_CBOR,
            value: serialize_vec(&value, "event value")?,
        });
        Ok(self)
    }

    /// Returns an actor event ready to emit (consuming self).
    pub fn build(self) -> ActorEvent {
        ActorEvent { entries: self.entries }
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
    use crate::{ActorError, EventBuilder};
    use fvm_shared::event::{ActorEvent, Entry, Flags};

    #[test]
    fn label() {
        let e = EventBuilder::new().label("l1").label("l2").build();
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
    fn values() -> Result<(), ActorError> {
        let e = EventBuilder::new().value("v1", &3)?.value_indexed("v2", "abc")?.build();
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

        Ok(())
    }
}
