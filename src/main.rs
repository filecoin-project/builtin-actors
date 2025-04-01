use anyhow::{anyhow, Context};
use clap::Parser;
use fil_actor_bundler::Bundler;
use fil_actors_runtime::runtime::builtins::Type;
use num_traits::cast::FromPrimitive;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ACTORS: &[(&str, &str)] = &[
    ("system", "system"),
    ("init", "init"),
    ("cron", "cron"),
    ("account", "account"),
    ("power", "storagepower"),
    ("miner", "storageminer"),
    ("market", "storagemarket"),
    ("paych", "paymentchannel"),
    ("multisig", "multisig"),
    ("reward", "reward"),
    ("verifreg", "verifiedregistry"),
    ("datacap", "datacap"),
    ("placeholder", "placeholder"),
    ("evm", "evm"),
    ("eam", "eam"),
    ("ethaccount", "ethaccount"),
];

const NETWORK_ENV: &str = "BUILD_FIL_NETWORK";

#[derive(Parser)]
#[clap(name = env!("CARGO_PKG_NAME"))]
#[clap(version = env!("CARGO_PKG_VERSION"))]
#[clap(about = "Builds and writes a CAR file containing Wasm bytecode for Filecoin actors.", long_about = None)]
struct Cli {
    /// The output car path
    #[clap(short, long, required = true)]
    output: PathBuf,

    /// Network name to build for: mainnet, calibrationnet, etc.
    #[clap(short, long, default_value = "mainnet")]
    network: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct CargoMessage {
    reason: String,
    target: Option<CargoTarget>,
    #[serde(default)]
    filenames: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CargoTarget {
    name: String,
}

fn build_bundle(network_name: &str, output_path: &Path) -> anyhow::Result<()> {
    // Cargo executable location
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    println!("Using cargo: {:?}", &cargo);

    // Compute the package names
    let packages =
        ACTORS.iter().map(|(pkg, _)| format!("fil_actor_{pkg}")).collect::<Vec<String>>();

    println!("Building for network: {}", network_name);

    // Cargo build command for all actors at once
    let mut cmd = Command::new(&cargo);
    cmd.arg("build")
        .args(packages.iter().map(|pkg| "-p=".to_owned() + pkg))
        .arg("--target=wasm32-unknown-unknown")
        .arg("--profile=wasm")
        .arg("--locked")
        .arg("--features=fil-actor")
        .arg("--message-format=json-render-diagnostics")
        .env(NETWORK_ENV, network_name)
        .stdout(Stdio::piped()) // json output.
        .stderr(Stdio::inherit()); // status messages

    println!("Running: {:?}", &cmd);

    // Run the build command.
    let result = cmd.output().context("failed to build the actors")?;
    if !result.status.success() {
        return Err(anyhow!("actor build failed"));
    }

    // Collect the actor bytecode.
    let mut actor_bytecode: HashMap<&str, Option<PathBuf>> =
        packages.iter().map(|n| (&**n, None)).collect();
    let messages = serde_json::Deserializer::from_slice(&result.stdout).into_iter::<CargoMessage>();
    for m in messages {
        let m = m.context("invalid cargo message format")?;
        if m.reason != "compiler-artifact" {
            continue;
        }

        let Some(pkg_name) = m.target.map(|t| t.name) else { continue };
        let Some(fname) = m.filenames.iter().find(|f| f.ends_with(".wasm")) else { continue };
        let Some(entry) = actor_bytecode.get_mut(&*pkg_name) else { continue };

        if let Some(existing) = entry {
            return Err(anyhow!(
                "duplicate artifact for {}: {} and {}",
                pkg_name,
                fname,
                existing.display(),
            ));
        }
        *entry = Some(fname.into());
    }

    let mut bundler = Bundler::new(output_path);
    for (&(pkg, name), id) in ACTORS.iter().zip(1u32..) {
        assert_eq!(
            name,
            Type::from_u32(id).expect("Type not defined").name(),
            "Actor types don't match actors included in the bundle"
        );
        let bytecode_path = actor_bytecode
            .get(&*format!("fil_actor_{pkg}"))
            .unwrap() // we always have an entry in the map.
            .as_ref()
            .with_context(|| format!("failed to build {pkg}"))?;

        // This actor version doesn't force synthetic CIDs; it uses genuine
        // content-addressed CIDs.
        let forced_cid = None;

        let cid = bundler
            .add_from_file(id, name.to_owned(), forced_cid, bytecode_path)
            .unwrap_or_else(|err| {
                panic!("Failed to add file {:?} to bundle for actor {}: {}", bytecode_path, id, err)
            });
        println!("Added {} ({}) to bundle with CID {}", name, id, cid);
    }
    bundler.finish().expect("Failed to finish bundle");

    println!("Bundle created at: {}", output_path.display());

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    build_bundle(&cli.network, &cli.output)
}
