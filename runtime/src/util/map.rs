use crate::builtin::HAMT_BIT_WIDTH;
use crate::{ActorError, AsActorError, Hasher};
use anyhow::anyhow;
use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_hamt as hamt;
use fvm_shared::error::ExitCode;
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Wraps a HAMT to provide a convenient map API.
/// The key type is Vec<u8>, so conversion to/from interpretations must by done by the caller.
/// Any errors are returned with exit code indicating illegal state.
/// The name is not persisted in state, but adorns any error messages.
pub struct Map2<'bs, BS, V>
where
    BS: Blockstore,
    V: DeserializeOwned + Serialize,
{
    hamt: hamt::Hamt<&'bs BS, V, hamt::BytesKey, Hasher>,
    name: &'static str,
}

trait MapKey: Sized {
    fn from_bytes(b: &[u8]) -> Result<Self, String>;
    fn to_bytes(&self) -> Result<Vec<u8>, String>;
}

pub type Config = hamt::Config;

pub const DEFAULT_CONF: Config =
    Config { bit_width: HAMT_BIT_WIDTH, min_data_depth: 0, max_array_width: 3 };

impl<'bs, BS, V> Map2<'bs, BS, V>
where
    BS: Blockstore,
    V: DeserializeOwned + Serialize,
{
    /// Creates a new, empty map.
    pub fn empty(store: &'bs BS, config: Config, name: &'static str) -> Self {
        Self { hamt: hamt::Hamt::new_with_config(store, config), name }
    }

    /// Creates a new empty map and flushes it to the store.
    /// Returns the CID of the empty map root.
    pub fn flush_empty(store: &'bs BS, config: Config) -> Result<Cid, ActorError> {
        // This CID is constant regardless of the HAMT's configuration, so as an optimisation
        // we could hard-code it and merely check it is already stored.
        Self::empty(store, config, "empty").flush()
    }

    /// Loads a map from the store.
    // There is no version of this method that doesn't take an explicit config parameter.
    // The caller must know the configuration to interpret the HAMT correctly.
    // Forcing them to provide it makes it harder to accidentally use an incorrect default.
    pub fn load(
        store: &'bs BS,
        root: &Cid,
        config: Config,
        name: &'static str,
    ) -> Result<Self, ActorError> {
        Ok(Self {
            hamt: hamt::Hamt::load_with_config(root, store, config)
                .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                    format!("failed to load HAMT '{}'", name)
                })?,
            name,
        })
    }

    /// Flushes the map's contents to the store.
    /// Returns the root node CID.
    pub fn flush(&mut self) -> Result<Cid, ActorError> {
        self.hamt.flush().with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
            format!("failed to flush HAMT '{}'", self.name)
        })
    }

    /// Returns a reference to the value associated with a key, if present.
    pub fn get(&self, key: &[u8]) -> Result<Option<&V>, ActorError> {
        self.hamt.get(key).with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
            format!("failed to get from HAMT '{}'", self.name)
        })
    }

    /// Inserts a key-value pair into the map.
    /// Returns any value previously associated with the key.
    pub fn set(&mut self, key: &[u8], value: V) -> Result<Option<V>, ActorError>
    where
        V: PartialEq,
    {
        self.hamt.set(key.into(), value).with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
            format!("failed to set in HAMT '{}'", self.name)
        })
    }

    /// Inserts a key-value pair only if the key does not already exist.
    /// Returns whether the map was modified (i.e. key was absent).
    pub fn set_if_absent(&mut self, key: &[u8], value: V) -> Result<bool, ActorError>
    where
        V: PartialEq,
    {
        self.hamt
            .set_if_absent(key.into(), value)
            .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                format!("failed to set in HAMT '{}'", self.name)
            })
    }

    /// Iterates over all key-value pairs in the map.
    pub fn for_each<F>(&self, mut f: F) -> Result<(), ActorError>
    where
        // Note the result type of F uses ActorError.
        // The implementation will extract and propagate any ActorError
        // wrapped in a hamt::Error::Dynamic.
        F: FnMut(&[u8], &V) -> Result<(), ActorError>,
    {
        match self.hamt.for_each(|k, v| f(k, v).map_err(|e| anyhow!(e))) {
            Ok(_) => Ok(()),
            Err(hamt_err) => match hamt_err {
                hamt::Error::Dynamic(e) => match e.downcast::<ActorError>() {
                    Ok(ae) => Err(ae),
                    Err(e) => Err(ActorError::illegal_state(format!(
                        "error traversing HAMT {}: {}",
                        self.name, e
                    ))),
                },
                e => Err(ActorError::illegal_state(format!(
                    "error traversing HAMT {}: {}",
                    self.name, e
                ))),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fvm_ipld_blockstore::MemoryBlockstore;

    #[test]
    fn basic_put_get() {
        let bs = MemoryBlockstore::new();
        let mut m = Map2::<MemoryBlockstore, String>::empty(&bs, DEFAULT_CONF, "empty");
        m.set(&[1, 2, 3, 4], "1234".to_string()).unwrap();
        assert!(m.get(&[1, 2]).unwrap().is_none());
        assert_eq!(&"1234".to_string(), m.get(&[1, 2, 3, 4]).unwrap().unwrap());
    }

    #[test]
    fn for_each_callback_exitcode_propagates() {
        let bs = MemoryBlockstore::new();
        let mut m = Map2::<MemoryBlockstore, String>::empty(&bs, DEFAULT_CONF, "empty");
        m.set(&[1, 2, 3, 4], "1234".to_string()).unwrap();
        let res = m.for_each(|_, _| Err(ActorError::forbidden("test".to_string())));
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), ActorError::forbidden("test".to_string()));
    }
}
