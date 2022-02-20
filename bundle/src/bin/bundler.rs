///! A Wasm bytecode CAR bundling utility. See helptext of command for docs.
use async_std::channel::bounded;
use async_std::task;
use ipld_car::CarHeader;
use std::error::Error;
use std::fs::File;
use std::io::Write;
use std::path::Path;

use async_std::task::block_on;
use cid::multihash::{Code, MultihashDigest};
use cid::Cid;
use clap::Parser;
use fvm_shared::blockstore::{Block, Blockstore, MemoryBlockstore};

const IPLD_RAW: u64 = 0x55;

#[derive(Parser)]
#[clap(name = "bundler")]
#[clap(version = "1.0")]
#[clap(about = "The bundle tool generates a CAR file containing Wasm bytecode
for Filecoin actors.

It takes a comma-separated list of bytecode paths to bundle (--bytecode-paths),
and the destination path of the bundle (--bundle-dst).

By default, this tool computes the CIDs of the bytecodes using the IPLD_RAW
multicodec (0x55) and a Blake2b256 multihash.

You may override CID generation by supplying a prefix (--override-cids-prefix)
and a comma-separated list of actor names (--actor-names). In that case, the
tool will concatenate the prefix and the actor name, and will use a CID with
the IPLD_RAW multicodec (0x55) and an Identity hash. This feature is useful
when looking to preserve pre-FVM mainnet compatibility.

This tool can also generate a CSV manifest of actor name => CID, to use as a
lookup table or reference. Provide the destination path of the manifest
(--manifest-dst) and a list of actor names (--actor-names). 

In all cases, when providing a list of actor names (--actor-names), its length
must match the length of the paths being bundled. ", long_about = None)]
struct Cli {
    /// The paths of the Wasm bytecode files to bundle.
    #[clap(long, required = true, multiple_values = true)]
    bytecode_paths: Vec<String>,
    /// Actor names to include their CIDs in the manifest.
    #[clap(long, multiple_values = true)]
    actor_names: Vec<String>,
    /// Overrides the CIDs of the bundled actors with synthetic CIDs, for
    /// compatibility with networks prior to FIP-0031.
    /// Provide the CID prefix, e.g. /fil/7 as a value.
    #[clap(long, requires = "actor-names")]
    override_cids_prefix: Option<String>,
    /// Path of the destination file.
    #[clap(long)]
    bundle_dst: String,
    /// Generate a manifest at the specified path, using the actor names specified in actor_names.
    #[clap(long, requires = "actor-names")]
    manifest_dst: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    // If we're outputting synthetic CIDs or a manifest, actor names must be
    // provided, and their length must be equal to the number of paths to bundle.
    let synthetic_cids = cli.override_cids_prefix.is_some();
    let manifest = cli.manifest_dst.is_some();
    if (manifest || synthetic_cids) && cli.actor_names.len() != cli.bytecode_paths.len() {
        return Err("number of actor names should be equal to number of paths".into());
    }

    // Create a memory blockstore and add all bytecodes to it, getting their CIDs.
    // NOTE: technically, the blockstore is not necessary since we're just writing flat blobs to
    // the CAR. But in the future, we will chunk these bytecodes into DAGs, and the blockstore
    // will be necessary.
    let bs = MemoryBlockstore::new();
    let bytecodes = cli.bytecode_paths.iter().map(|path| {
        let data = std::fs::read(path).expect(format!("failed to read file: {}", path).as_str());
        (path.clone(), data)
    });

    // Add bytecodes to the blockstore.
    let cids: Vec<Cid> = if synthetic_cids {
        with_synthetic_cids(
            &bs,
            cli.override_cids_prefix.unwrap().as_ref(),
            &cli.actor_names,
            bytecodes,
        )
    } else {
        with_real_cids(&bs, bytecodes)
    };

    // Create a CAR file containing all bytecodes.
    block_on(write_car(bs, &cids, &cli.bundle_dst))?;

    // Optionally, create a manifest of CIDs bundled.
    if manifest {
        let mut manifest = File::create(Path::new(&cli.manifest_dst.unwrap()))?;
        for (name, cid) in cli.actor_names.iter().zip(cids.iter()) {
            write!(manifest, "{},{}\n", name, cid.to_string())?;
        }
        manifest.flush()?;
    }

    Ok(())
}

fn with_real_cids(
    bs: &MemoryBlockstore,
    bytecodes: impl Iterator<Item = (String, Vec<u8>)>,
) -> Vec<Cid> {
    bytecodes
        .map(|(path, bytecode)| {
            bs.put(
                Code::Blake2b256,
                &Block {
                    codec: IPLD_RAW,
                    data: bytecode,
                },
            )
            .expect(format!("failed to put bytecode in {} into blockstore", path).as_str())
        })
        .collect()
}

fn with_synthetic_cids(
    bs: &MemoryBlockstore,
    prefix: &str,
    actor_names: &Vec<String>,
    bytecodes: impl Iterator<Item = (String, Vec<u8>)>,
) -> Vec<Cid> {
    let synthetic_cids = actor_names.iter().map(|name| {
        let identity = prefix.to_owned() + name.as_ref();
        Cid::new_v1(IPLD_RAW, Code::Identity.digest(identity.as_bytes()))
    });
    bytecodes
        .zip(synthetic_cids)
        .map(|((path, bytecode), cid)| {
            bs.put_keyed(&cid, &bytecode)
                .expect(format!("failed to put bytecode in {} into blockstore", path).as_str());
            cid
        })
        .collect()
}

async fn write_car(bs: MemoryBlockstore, cids: &[Cid], dst: &str) -> std::io::Result<()> {
    let mut out = async_std::fs::File::create(dst).await?;

    let car = CarHeader {
        roots: cids.to_owned(),
        version: 1,
    };

    let (tx, mut rx) = bounded(16);

    let write_task =
        task::spawn(async move { car.write_stream_async(&mut out, &mut rx).await.unwrap() });

    for cid in cids.iter() {
        println!("adding cid {} to bundle CAR", cid.to_string());
        let bytecode = bs.get(cid).unwrap().unwrap();
        tx.send((*cid, bytecode)).await.unwrap();
    }

    drop(tx);

    write_task.await;

    Ok(())
}
