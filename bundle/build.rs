///! This build script generates two files in the Cargo OUT_DIR:
///
/// - bundle.car: a CAR file containing the bytecode of the canonical actors.
///   Its has a multiroot header, enumerating the CID of every actor bytecode.
///   Each bytecode entry is encoded as a single IPLD slab; there is no DAG
///   being formed for now. This may change in the future.
/// - manifest: a comma-separated text file containing a manifest of actors and
///   their CIDs.
///
/// Because Cargo randomizes the OUT_DIR (at least on my tested platforms), this
/// solution is less than ideal. We need to find a way to output to a stable path.
use async_std::channel::bounded;
use async_std::task;
use async_std::task::block_on;
use cid::multihash::Code;
use cid::Cid;
use fvm_shared::blockstore::{Block, Blockstore, MemoryBlockstore};
use ipld_car::CarHeader;
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fs::File;
use std::io::Write;
use std::path::Path;

const IPLD_RAW: u64 = 0x55;

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir = env::var_os("OUT_DIR").unwrap();

    // 1. Collect all wasm bytecodes to bundle.
    let actors: Vec<(&str, &[u8])> = vec![
        (
            "account",
            fvm_actor_account::wasm::WASM_BINARY_BLOATY.unwrap(),
        ),
        ("cron", fvm_actor_cron::wasm::WASM_BINARY_BLOATY.unwrap()),
        ("init", fvm_actor_init::wasm::WASM_BINARY_BLOATY.unwrap()),
        (
            "market",
            fvm_actor_market::wasm::WASM_BINARY_BLOATY.unwrap(),
        ),
        ("miner", fvm_actor_miner::wasm::WASM_BINARY_BLOATY.unwrap()),
        (
            "multisig",
            fvm_actor_multisig::wasm::WASM_BINARY_BLOATY.unwrap(),
        ),
        ("paych", fvm_actor_paych::wasm::WASM_BINARY_BLOATY.unwrap()),
        ("power", fvm_actor_power::wasm::WASM_BINARY_BLOATY.unwrap()),
        (
            "reward",
            fvm_actor_reward::wasm::WASM_BINARY_BLOATY.unwrap(),
        ),
        (
            "system",
            fvm_actor_system::wasm::WASM_BINARY_BLOATY.unwrap(),
        ),
        (
            "verifreg",
            fvm_actor_verifreg::wasm::WASM_BINARY_BLOATY.unwrap(),
        ),
    ];

    // 2. Create a memory blockstore and add all bytecodes to it, getting their CIDs.
    let bs = MemoryBlockstore::new();
    let cids: Vec<(&str, Cid)> = actors
        .iter()
        .map(|(name, bytecode)| {
            let blk = &Block {
                codec: IPLD_RAW,
                data: bytecode,
            };
            let cid = bs.put(Code::Blake2b256, blk).expect(
                format!("failed to put bytecode of actor {} into blockstore", name).as_str(),
            );
            (*name, cid)
        })
        .collect();

    // 3. Create a CAR file containing all bytecodes.
    block_on(write_car(&out_dir, &actors, &cids))?;

    // 4. Create a manifest of CIDs bundled.
    let mut manifest = File::create(Path::new(&out_dir).join("manifest"))?;
    for (name, cid) in &cids {
        write!(manifest, "{},{}\n", name, cid.to_string())?;
    }
    manifest.flush()?;
    Ok(())
}

async fn write_car(
    out_dir: &OsString,
    actors: &Vec<(&str, &[u8])>,
    cids: &Vec<(&str, Cid)>,
) -> std::io::Result<()> {
    let mut out = async_std::fs::File::create(Path::new(&out_dir).join("bundle.car")).await?;

    let car = CarHeader {
        roots: cids.iter().map(|(_, cid)| *cid).collect(),
        version: 1,
    };

    let (tx, mut rx) = bounded(16);

    let write_task =
        task::spawn(async move { car.write_stream_async(&mut out, &mut rx).await.unwrap() });

    for ((name, cid), (_, bytecode)) in cids.iter().zip(actors.iter()) {
        println!("adding {} with cid {}", name, cid.to_string());
        tx.send((*cid, bytecode.to_vec())).await.unwrap();
    }

    drop(tx);

    write_task.await;

    Ok(())
}
