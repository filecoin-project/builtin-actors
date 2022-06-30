use std::marker::PhantomData;

use cid::{multihash::Code, Cid};

mod amt;
mod hamt;
mod link;
pub use amt::TAmt;
pub use hamt::THamt;
pub use link::TLink;

/// Helper type to be able to define `Code` as a generic parameter.
pub trait CodeType {
    fn code() -> Code;
}

/// Marker trait for types that were meant to be used inside a TCid.
pub trait TCidContent {}

/// `TCid` is typed content, represented by a `Cid`.
#[derive(PartialEq, Eq, Clone, Debug)]
pub struct TCid<T: TCidContent, C = codes::Blake2b256> {
    cid: Cid,
    _phantom_t: PhantomData<T>,
    _phantom_c: PhantomData<C>,
}

impl<T: TCidContent, C: CodeType> TCid<T, C> {
    pub fn cid(&self) -> Cid {
        self.cid
    }
    pub fn code(&self) -> Code {
        C::code()
    }
}

impl<T: TCidContent, C> From<Cid> for TCid<T, C> {
    fn from(cid: Cid) -> Self {
        TCid { cid, _phantom_t: PhantomData, _phantom_c: PhantomData }
    }
}

/// Serializes exactly as its underlying `Cid`.
impl<T: TCidContent, C> serde::Serialize for TCid<T, C> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.cid.serialize(serializer)
    }
}

/// Deserializes exactly as its underlying `Cid`.
impl<'d, T: TCidContent, C> serde::Deserialize<'d> for TCid<T, C> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'d>,
    {
        let cid = Cid::deserialize(deserializer)?;
        Ok(Self::from(cid))
    }
}

/// Assuming that the type implements `maybe_load` and `flush`,
/// implement some convenience methods.
///
/// NOTE: This can be achieved with a trait and an associated type as well, but unfortunately
/// it got too complex for Rust Analyzer to provide code completion, which makes it less ergonomic.
/// At least this way there's no need to import the trait that contains these ops.
#[macro_export]
macro_rules! tcid_ops {
    (
        $typ:ident <
          $($gen:ident $($const:ident)? $(: $b:ident $(+ $bs:ident)* )? ),+
        >
        $(, $code:ident : $ct:ident)?
        => $item:ty
    ) => {
        /// Operations on content types that, once loaded, are rooted
        /// and bound to a block store, and need to be flushed back.
        impl<
          $($($const)? $gen $(: $b $(+ $bs)* )? ),+
          $(, $code : $ct)?
        > TCid<$typ<$($gen),+> $(, $code)?>
        {
            /// Read the underlying `Cid` from the store or return a `ActorError::illegal_state` error if not found.
            /// Use this method for content that should have already been correctly initialized and maintained.
            /// For content that may not be present, consider using `maybe_load` instead.
            pub fn load<'s, S: fvm_ipld_blockstore::Blockstore>(&self, store: &'s S) -> Result<$item> {
                match self.maybe_load(store)? {
                    Some(content) => Ok(content),
                    None => Err(fil_actors_runtime::actor_error!(
                        illegal_state;
                        "error loading {}: Cid ({}) did not match any in database",
                        type_name::<Self>(),
                        self.cid.to_string()
                    ).into()),
                }
            }

            /// Load, modify and flush a value, returning something as a result.
            pub fn modify<'s, S: fvm_ipld_blockstore::Blockstore, R>(
                &mut self,
                store: &'s S,
                f: impl FnOnce(&mut $item) -> anyhow::Result<R>,
            ) -> anyhow::Result<R> {
                let mut value = self.load(store)?;
                let result = f(&mut value)?;
                self.flush(value)?;
                Ok(result)
            }

            /// Load, modify and flush a value.
            pub fn update<'s, S: fvm_ipld_blockstore::Blockstore>(
                &mut self,
                store: &'s S,
                f: impl FnOnce(&mut $item) -> anyhow::Result<()>,
            ) -> anyhow::Result<()> {
                self.modify(store, f)
            }
        }
    }
}

pub mod codes {
    use super::CodeType;

    /// Define a unit struct for a `Code` element that
    /// can be used as a generic parameter.
    macro_rules! code_types {
        ($($code:ident => $typ:ident),+) => {
            $(
            #[derive(PartialEq, Eq, Clone, Debug)]
            pub struct $typ;

            impl CodeType for $typ {
                fn code() -> cid::multihash::Code {
                    cid::multihash::Code::$code
                }
            }
            )*
        };
    }

    // XXX: For some reason none of the other code types work,
    // not even on their own as a variable:
    // let c = multihash::Code::Keccak256;
    // ERROR: no variant or associated item named `Keccak256` found for enum `Code`
    //        in the current scope variant or associated item not found in `Code`
    code_types! {
      Blake2b256 => Blake2b256
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use cid::Cid;
    use fvm_ipld_blockstore::MemoryBlockstore;
    use fvm_ipld_encoding::tuple::*;
    use fvm_ipld_hamt::BytesKey;

    #[derive(Default, Serialize_tuple, Deserialize_tuple, PartialEq)]
    struct TestRecord {
        foo: u64,
        bar: Vec<u8>,
    }

    #[derive(Default, Serialize_tuple, Deserialize_tuple)]
    struct TestRecordTyped {
        pub optional: Option<TCid<TLink<TestRecord>>>,
        pub map: TCid<THamt<String, TestRecord>>,
    }

    impl TestRecordTyped {
        fn new(store: &MemoryBlockstore) -> Self {
            Self { optional: None, map: TCid::new_hamt(store).unwrap() }
        }
    }

    #[derive(Default, Serialize_tuple, Deserialize_tuple)]
    struct TestRecordUntyped {
        pub optional: Option<Cid>,
        pub map: Cid,
    }

    #[test]
    fn default_cid_and_default_hamt_differ() {
        let cid_typed: TCid<TLink<TestRecordTyped>> = TCid::default();
        let cid_untyped: TCid<TLink<TestRecordUntyped>> = TCid::default();
        // The stronger typing allows us to use proper default values,
        // but this is a breaking change from the invalid values that came before.
        assert_ne!(cid_typed.cid(), cid_untyped.cid());
    }

    #[test]
    fn default_value_read_fails() {
        let cid_typed: TCid<TLink<TestRecordTyped>> = TCid::default();
        let store = MemoryBlockstore::new();
        assert!(cid_typed.load(&store).is_err());
    }

    #[test]
    fn ref_modify() {
        let store = MemoryBlockstore::new();
        let mut r: TCid<TLink<TestRecord>> =
            TCid::new_link(&store, &TestRecord::default()).unwrap();

        r.modify(&store, |c| {
            c.foo += 1;
            Ok(())
        })
        .unwrap();

        assert_eq!(r.load(&store).unwrap().foo, 1);
    }

    #[test]
    fn hamt_modify() {
        let store = MemoryBlockstore::new();
        let mut rec = TestRecordTyped::new(&store);

        let eggs = rec
            .map
            .modify(&store, |map| {
                map.set(BytesKey::from("spam"), TestRecord { foo: 1, bar: Vec::new() })?;
                Ok("eggs")
            })
            .unwrap();
        assert_eq!(eggs, "eggs");

        let map = rec.map.load(&store).unwrap();

        let foo = map.get(&BytesKey::from("spam")).unwrap().map(|x| x.foo);
        assert_eq!(foo, Some(1))
    }
}
