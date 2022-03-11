///! A Wasm bytecode CAR bundling utility. See helptext of command for docs.
use std::error::Error;

use cid::multihash::Multihash;
use cid::Cid;
use clap::Parser;
use fil_actor_bundler::Bundler;

const IPLD_RAW: u64 = 0x55;

#[derive(Parser)]
#[clap(name = "bundler")]
#[clap(version = "1.0")]
#[clap(about = "The bundle tool generates a CAR file containing Wasm bytecode
for Filecoin actors.

It takes a comma-separated list of bytecode paths to bundle (--bytecode-paths)
and their corresponing actor names (--actor-names). It then generates two
artifacts: a CAR bundle and a manifest, in their respective paths (--bundle-dst,
--manifest-dst).

By default, this tool computes the CIDs of the bytecodes using the IPLD_RAW
multicodec (0x55) and a Blake2b256 multihash. You may override CID generation
by supplying a prefix (--override-cids-prefix). In that case, the
tool will concatenate the prefix and the actor name, and will use a CID with
the IPLD_RAW multicodec (0x55) and an Identity hash. This feature is useful
when looking to preserve pre-FVM mainnet compatibility.

The length of the list of actor names (--actor-names) must match the length of
the paths being bundled.", long_about = None)]
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
    #[clap(long)]
    override_cids_prefix: Option<String>,
    /// Path of the destination file.
    #[clap(long)]
    bundle_dst: String,
    /// Generate a manifest at the specified path, using the actor names specified in actor_names.
    #[clap(long)]
    manifest_dst: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    if cli.actor_names.len() != cli.bytecode_paths.len() {
        return Err("number of actor names should be equal to number of paths".into());
    }

    let mut bundler = Bundler::new(cli.bundle_dst);
    for (path, name) in cli.bytecode_paths.into_iter().zip(cli.actor_names.iter()) {
        let cid = cli
            .override_cids_prefix
            .as_ref()
            .map(|prefix| {
                let identity = prefix.to_owned() + name.as_ref();
                Multihash::wrap(0, identity.as_bytes()).map(|mh| Cid::new_v1(IPLD_RAW, mh))
            })
            .transpose()?;
        let cid = bundler.add_from_file(name.as_str().try_into().unwrap(), cid.as_ref(), path)?;
        println!("added actor {} with CID {}", name, cid)
    }

    bundler.finish()?;

    Ok(())
}
