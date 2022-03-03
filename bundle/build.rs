use cid::multihash::{Code, MultihashDigest};
use cid::Cid;
use fil_actor_bundler::Bundler;
use std::error::Error;
use std::path::{Path, PathBuf};

/// Technical identifier for the actor in legacy CodeCIDs and else.
type ID = str;

const ACTORS: &[(&ID, &[u8])] = &[
    ("system", fil_actor_system::WASM_BINARY),
    ("init", fil_actor_init::WASM_BINARY),
    ("cron", fil_actor_cron::WASM_BINARY),
    ("account", fil_actor_account::WASM_BINARY),
    ("multisig", fil_actor_multisig::WASM_BINARY),
    ("storagepower", fil_actor_power::WASM_BINARY),
    ("storageminer", fil_actor_miner::WASM_BINARY),
    ("storagemarket", fil_actor_market::WASM_BINARY),
    ("paymentchannel", fil_actor_paych::WASM_BINARY),
    ("reward", fil_actor_reward::WASM_BINARY),
    ("verifiedregistry", fil_actor_verifreg::WASM_BINARY),
];

const IPLD_RAW: u64 = 0x55;
const FORCED_CID_PREFIX: &str = "fil/6/";

fn main() -> Result<(), Box<dyn Error>> {
    let out_dir: PathBuf = std::env::var_os("OUT_DIR")
        .expect("no OUT_DIR env var")
        .into();

    let dst = Path::new(&out_dir).join("bundle.car");
    let mut bundler = Bundler::new(&dst);
    for &(id, bytecode) in ACTORS {
        // This actor version uses forced CIDs.
        let forced_cid = {
            let identity = FORCED_CID_PREFIX.to_owned() + id;
            Cid::new_v1(IPLD_RAW, Code::Identity.digest(identity.as_bytes()))
        };

        let cid = bundler
            .add_from_bytes((*id).try_into().unwrap(), Some(&forced_cid), bytecode)
            .unwrap_or_else(|err| panic!("failed to add actor {} to bundle: {}", id, err));
        println!(
            "cargo:warning=added actor {} to bundle with CID {}",
            id, cid
        );
    }
    bundler.finish().expect("failed to finish bundle");

    println!("cargo:warning=bundle={}", dst.display());
    println!(
        "cargo:rustc-env=FIL_ACTOR_BUNDLE={}",
        dst.display().to_string().escape_default()
    );

    Ok(())
}
