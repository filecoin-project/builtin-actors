use crate::builtin::HAMT_BIT_WIDTH;
use crate::{ActorError, AsActorError, Hasher};
use anyhow::anyhow;
use cid::Cid;
use fvm_ipld_blockstore::Blockstore;
use fvm_ipld_hamt as hamt;
use fvm_shared::address::Address;
use fvm_shared::error::ExitCode;
use integer_encoding::VarInt;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt::Debug;
use std::marker::PhantomData;

/// Wraps a HAMT to provide a convenient map API.
/// Any errors are returned with exit code indicating illegal state.
/// The name is not persisted in state, but adorns any error messages.
pub struct Map2<BS, K, V>
where
    BS: Blockstore,
    K: MapKey,
    V: DeserializeOwned + Serialize,
{
    hamt: hamt::Hamt<BS, V, hamt::BytesKey, Hasher>,
    name: &'static str,
    key_type: PhantomData<K>,
}

pub trait MapKey: Sized + Debug {
    fn from_bytes(b: &[u8]) -> Result<Self, String>;
    fn to_bytes(&self) -> Result<Vec<u8>, String>;
}

pub type Config = hamt::Config;

pub const DEFAULT_HAMT_CONFIG: Config =
    Config { bit_width: HAMT_BIT_WIDTH, min_data_depth: 0, max_array_width: 3 };

impl<BS, K, V> Map2<BS, K, V>
where
    BS: Blockstore,
    K: MapKey,
    V: DeserializeOwned + Serialize,
{
    /// Creates a new, empty map.
    pub fn empty(store: BS, config: Config, name: &'static str) -> Self {
        Self {
            hamt: hamt::Hamt::new_with_config(store, config),
            name,
            key_type: Default::default(),
        }
    }

    /// Creates a new empty map and flushes it to the store.
    /// Returns the CID of the empty map root.
    pub fn flush_empty(store: BS, config: Config) -> Result<Cid, ActorError> {
        // This CID is constant regardless of the HAMT's configuration, so as an optimisation
        // we could hard-code it and merely check it is already stored.
        Self::empty(store, config, "empty").flush()
    }

    /// Loads a map from the store.
    // There is no version of this method that doesn't take an explicit config parameter.
    // The caller must know the configuration to interpret the HAMT correctly.
    // Forcing them to provide it makes it harder to accidentally use an incorrect default.
    pub fn load(
        store: BS,
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
            key_type: Default::default(),
        })
    }

    /// Flushes the map's contents to the store.
    /// Returns the root node CID.
    pub fn flush(&mut self) -> Result<Cid, ActorError> {
        self.hamt.flush().with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
            format!("failed to flush HAMT '{}'", self.name)
        })
    }

    /// Returns a reference to the underlying blockstore.
    pub fn store(&self) -> &BS {
        self.hamt.store()
    }

    /// Returns whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.hamt.is_empty()
    }

    /// Returns a reference to the value associated with a key, if present.
    pub fn get(&self, key: &K) -> Result<Option<&V>, ActorError> {
        let k = key.to_bytes().context_code(ExitCode::USR_ASSERTION_FAILED, "invalid key")?;
        self.hamt.get(&k).with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
            format!("failed to get key {key:?} from HAMT '{}'", self.name)
        })
    }

    pub fn contains_key(&self, key: &K) -> Result<bool, ActorError> {
        let k = key.to_bytes().context_code(ExitCode::USR_ASSERTION_FAILED, "invalid key")?;
        self.hamt.contains_key(&k).with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
            format!("failed to check key {key:?} in HAMT '{}'", self.name)
        })
    }

    /// Inserts a key-value pair into the map.
    /// Returns any value previously associated with the key.
    pub fn set(&mut self, key: &K, value: V) -> Result<Option<V>, ActorError>
    where
        V: PartialEq,
    {
        let k = key.to_bytes().context_code(ExitCode::USR_ASSERTION_FAILED, "invalid key")?;
        self.hamt.set(k.into(), value).with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
            format!("failed to set key {key:?} in HAMT '{}'", self.name)
        })
    }

    /// Inserts a key-value pair only if the key does not already exist.
    /// Returns whether the map was modified (i.e. key was absent).
    pub fn set_if_absent(&mut self, key: &K, value: V) -> Result<bool, ActorError>
    where
        V: PartialEq,
    {
        let k = key.to_bytes().context_code(ExitCode::USR_ASSERTION_FAILED, "invalid key")?;
        self.hamt
            .set_if_absent(k.into(), value)
            .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                format!("failed to set key {key:?} in HAMT '{}'", self.name)
            })
    }

    pub fn delete(&mut self, key: &K) -> Result<Option<V>, ActorError> {
        let k = key
            .to_bytes()
            .with_context_code(ExitCode::USR_ASSERTION_FAILED, || format!("invalid key {key:?}"))?;
        self.hamt
            .delete(&k)
            .map(|delete_result| delete_result.map(|(_k, v)| v))
            .with_context_code(ExitCode::USR_ILLEGAL_STATE, || {
                format!("failed to delete key {key:?} from HAMT '{}'", self.name)
            })
    }

    /// Iterates over all key-value pairs in the map.
    pub fn for_each<F>(&self, mut f: F) -> Result<(), ActorError>
    where
        // Note the result type of F uses ActorError.
        // The implementation will extract and propagate any ActorError
        // wrapped in a hamt::Error::Dynamic.
        F: FnMut(K, &V) -> Result<(), ActorError>,
    {
        self.hamt
            .for_each(|k, v| {
                let key =
                    K::from_bytes(k).context_code(ExitCode::USR_ILLEGAL_STATE, "invalid key")?;
                f(key, v).map_err(|e| anyhow!(e))
            })
            .map_err(|hamt_err| match hamt_err {
                hamt::Error::Dynamic(e) => match e.downcast::<ActorError>() {
                    Ok(ae) => ae,
                    Err(e) => ActorError::illegal_state(format!(
                        "error in callback traversing HAMT {}: {}",
                        self.name, e
                    )),
                },
                e => {
                    ActorError::illegal_state(format!("error traversing HAMT {}: {}", self.name, e))
                }
            })
    }
}

impl MapKey for Vec<u8> {
    fn from_bytes(b: &[u8]) -> Result<Self, String> {
        Ok(b.to_vec())
    }

    fn to_bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.clone())
    }
}

impl MapKey for u64 {
    fn from_bytes(b: &[u8]) -> Result<Self, String> {
        if let Some((result, size)) = VarInt::decode_var(b) {
            if size != b.len() {
                return Err(format!("trailing bytes after varint in {:?}", b));
            }
            Ok(result)
        } else {
            Err(format!("failed to decode varint in {:?}", b))
        }
    }

    fn to_bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.encode_var_vec())
    }
}

impl MapKey for i64 {
    fn from_bytes(b: &[u8]) -> Result<Self, String> {
        if let Some((result, size)) = VarInt::decode_var(b) {
            if size != b.len() {
                return Err(format!("trailing bytes after varint in {:?}", b));
            }
            Ok(result)
        } else {
            Err(format!("failed to decode varint in {:?}", b))
        }
    }

    fn to_bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.encode_var_vec())
    }
}

impl MapKey for Address {
    fn from_bytes(b: &[u8]) -> Result<Self, String> {
        Address::from_bytes(b).map_err(|e| e.to_string())
    }

    fn to_bytes(&self) -> Result<Vec<u8>, String> {
        Ok(Address::to_bytes(*self))
    }
}

impl MapKey for Cid {
    fn from_bytes(b: &[u8]) -> Result<Self, String> {
        Cid::try_from(b).map_err(|e| e.to_string())
    }

    fn to_bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_blockstores::MemoryBlockstore;

    #[test]
    fn basic_put_get() {
        let bs = MemoryBlockstore::new();
        let mut m = Map2::<_, u64, String>::empty(bs, DEFAULT_HAMT_CONFIG, "empty");
        m.set(&1234, "1234".to_string()).unwrap();
        assert!(m.get(&2222).unwrap().is_none());
        assert_eq!(&"1234".to_string(), m.get(&1234).unwrap().unwrap());
    }

    #[test]
    fn for_each_callback_exitcode_propagates() {
        let bs = MemoryBlockstore::new();
        let mut m = Map2::<_, u64, String>::empty(bs, DEFAULT_HAMT_CONFIG, "empty");
        m.set(&1234, "1234".to_string()).unwrap();
        let res = m.for_each(|_, _| Err(ActorError::forbidden("test".to_string())));
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), ActorError::forbidden("test".to_string()));
    }
}
