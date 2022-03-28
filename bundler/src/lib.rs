use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use async_std::channel::bounded;
use async_std::task;
use async_std::task::block_on;

use anyhow::Context;
use anyhow::Result;
use cid::multihash::Code;
use cid::Cid;
use fvm_ipld_car::CarHeader;
use fvm_shared::actor::builtin::Type as ActorType;
use fvm_shared::blockstore::{Block, Blockstore, MemoryBlockstore};
use fvm_shared::encoding::tuple::*;
use fvm_shared::encoding::Cbor;
use fvm_shared::encoding::DAG_CBOR;

const IPLD_RAW: u64 = 0x55;

/// A library to bundle the Wasm bytecode of builtin actors into a CAR file.
///
/// The single root CID of the CAR file points to an CBOR-encoded IPLD
/// Map<Cid, i32> where i32 is to be interpreted as an
/// fvm_shared::actor::builtin::Type enum value.
pub struct Bundler {
    /// Staging blockstore.
    blockstore: MemoryBlockstore,
    /// Tracks the mapping of actors to Cids. Inverted when writing. Allows overriding.
    added: BTreeMap<ActorType, Cid>,
    /// Path of the output bundle.
    bundle_dst: PathBuf,
}

impl Bundler {
    pub fn new<P>(bundle_dst: P) -> Bundler
    where
        P: AsRef<Path>,
    {
        Bundler {
            bundle_dst: bundle_dst.as_ref().to_owned(),
            blockstore: Default::default(),
            added: Default::default(),
        }
    }

    /// Adds bytecode from a byte slice.
    pub fn add_from_bytes(
        &mut self,
        actor_type: ActorType,
        forced_cid: Option<&Cid>,
        bytecode: &[u8],
    ) -> Result<Cid> {
        let cid = match forced_cid {
            Some(cid) => self.blockstore.put_keyed(cid, bytecode).and(Ok(*cid)),
            None => self.blockstore.put(
                Code::Blake2b256,
                &Block {
                    codec: IPLD_RAW,
                    data: bytecode,
                },
            ),
        }
        .with_context(|| {
            format!(
                "failed to put bytecode for actor {:?} into blockstore",
                actor_type
            )
        })?;
        self.added.insert(actor_type, cid);
        Ok(cid)
    }

    /// Adds bytecode from a file.
    pub fn add_from_file<P: AsRef<Path>>(
        &mut self,
        actor_type: ActorType,
        forced_cid: Option<&Cid>,
        bytecode_path: P,
    ) -> Result<Cid> {
        let bytecode = std::fs::read(bytecode_path).context("failed to open bytecode file")?;
        self.add_from_bytes(actor_type, forced_cid, bytecode.as_slice())
    }

    /// Commits the added bytecode entries and writes the CAR file to disk.
    pub fn finish(self) -> Result<()> {
        block_on(self.write_car())
    }

    async fn write_car(self) -> Result<()> {
        let mut out = async_std::fs::File::create(&self.bundle_dst).await?;

        let manifest_payload: Vec<(String, Cid)> =
            self.added.iter().map(|(t, c)| (t.into(), *c)).collect();
        let manifest_data = serde_ipld_dagcbor::to_vec(&manifest_payload)?;
        let manifest_link = self.blockstore.put(
            Code::Blake2b256,
            &Block {
                codec: DAG_CBOR,
                data: &manifest_data,
            },
        )?;

        let manifest = Manifest {
            version: 1,
            data: manifest_link,
        };
        let manifest_bytes = serde_ipld_dagcbor::to_vec(&manifest)?;

        let root = self.blockstore.put(
            Code::Blake2b256,
            &Block {
                codec: DAG_CBOR,
                data: &manifest_bytes,
            },
        )?;

        // Create a CAR header.
        let car = CarHeader {
            roots: vec![root],
            version: 1,
        };

        let (tx, mut rx) = bounded(16);
        let write_task =
            task::spawn(async move { car.write_stream_async(&mut out, &mut rx).await.unwrap() });

        // Add the root payload.
        tx.send((root, manifest_bytes)).await.unwrap();

        // Add the manifest payload.
        tx.send((manifest_link, manifest_data)).await.unwrap();

        // Add the bytecodes.
        for cid in self.added.iter().map(|(_, cid)| cid) {
            println!("adding cid {} to bundle CAR", cid);
            let data = self.blockstore.get(cid).unwrap().unwrap();
            tx.send((*cid, data)).await.unwrap();
        }

        drop(tx);

        write_task.await;

        Ok(())
    }
}

/// The Manifest struct is the versioned envelope for builtin actor manifests.
/// The only currently supported version is 1, which encodes the data as a tuple
/// of strings (actor names) and actor code cids.
#[derive(Serialize_tuple, Deserialize_tuple, Clone)]
pub struct Manifest {
    pub version: u32,
    pub data: Cid,
}

impl Cbor for Manifest {}

#[test]
fn test_bundler() {
    use async_std::fs::File;
    use cid::multihash::Multihash;
    use fvm_ipld_car::{load_car, CarReader};
    use num_traits::FromPrimitive;
    use rand::Rng;

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("test_bundle.car");

    // Write 10 random payloads to the bundle.
    let mut cids = Vec::with_capacity(10);
    let mut bundler = Bundler::new(&path);

    // First 5 have real CIDs, last 5 have forced CIDs.
    for i in 0..10 {
        let forced_cid = (i > 5).then(|| {
            // identity hash
            Cid::new_v1(
                IPLD_RAW,
                Multihash::wrap(0, format!("actor-{}", i).as_bytes()).unwrap(),
            )
        });
        let typ = ActorType::from_i32(i + 1).unwrap();
        let cid = bundler
            .add_from_bytes(
                typ,
                forced_cid.as_ref(),
                &rand::thread_rng().gen::<[u8; 32]>(),
            )
            .unwrap();

        dbg!(cid.to_string());
        cids.push(cid);
    }
    bundler.finish().unwrap();

    // Read with the CarReader directly and verify there's a single root.
    let reader = block_on(async {
        let file = File::open(&path).await.unwrap();
        CarReader::new(file).await.unwrap()
    });
    assert_eq!(reader.header.roots.len(), 1);
    dbg!(reader.header.roots[0].to_string());

    // Load the CAR into a Blockstore.
    let bs = MemoryBlockstore::default();
    let roots = block_on(async {
        let file = File::open(&path).await.unwrap();
        load_car(&bs, file).await.unwrap()
    });
    assert_eq!(roots.len(), 1);

    // Compare that the previous root matches this one.
    assert_eq!(reader.header.roots[0], roots[0]);

    // The single root represents the manifest.
    let manifest_cid = roots[0];
    let manifest_bytes = bs.get(&manifest_cid).unwrap().unwrap();

    // Deserialize the manifest.
    let manifest: Manifest = serde_ipld_dagcbor::from_slice(manifest_bytes.as_slice()).unwrap();
    assert_eq!(manifest.version, 1);

    let manifest_data = bs.get(&manifest.data).unwrap().unwrap();
    let manifest_vec: Vec<(String, Cid)> =
        serde_ipld_dagcbor::from_slice(manifest_data.as_slice()).unwrap();
    let manifest: BTreeMap<ActorType, Cid> = manifest_vec
        .iter()
        .map(|(s, c)| (ActorType::try_from(s.as_str()).unwrap(), *c))
        .collect();

    // Verify the manifest contains what we expect.
    for (i, cid) in cids.into_iter().enumerate() {
        let typ = ActorType::from_i32((i + 1) as i32).unwrap();
        assert_eq!(manifest.get(&typ).unwrap(), &cid);

        // Verify that the last 5 CIDs are really forced CIDs.
        if i > 5 {
            let expected = Cid::new_v1(
                IPLD_RAW,
                Multihash::wrap(0, format!("actor-{}", i).as_bytes()).expect("name too long"),
            );
            assert_eq!(cid, expected)
        }
        assert!(bs.has(&cid).unwrap());
    }
}
