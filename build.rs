use cid::multihash::Multihash;
use cid::Cid;
use fil_actor_bundler::Bundler;
use std::error::Error;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

/// Cargo package for an actor.
type Package = str;

/// Technical identifier for the actor in legacy CodeCIDs and else.
type ID = str;

const ACTORS: &[(&Package, &ID)] = &[
    ("system", "system"),
    ("init", "init"),
    ("cron", "cron"),
    ("account", "account"),
    ("multisig", "multisig"),
    ("power", "storagepower"),
    ("miner", "storageminer"),
    ("market", "storagemarket"),
    ("paych", "paymentchannel"),
    ("reward", "reward"),
    ("verifreg", "verifiedregistry"),
];

const IPLD_RAW: u64 = 0x55;
const FORCED_CID_PREFIX: &str = "fil/7/";

fn main() -> Result<(), Box<dyn Error>> {
    // Cargo executable location.
    let cargo = std::env::var_os("CARGO").expect("no CARGO env var");
    println!("cargo:warning=cargo: {:?}", &cargo);

    let out_dir = std::env::var_os("OUT_DIR")
        .as_ref()
        .map(Path::new)
        .map(|p| p.join("bundle"))
        .expect("no OUT_DIR env var");
    println!("cargo:warning=out_dir: {:?}", &out_dir);

    // Compute the package names.
    let packages =
        ACTORS.iter().map(|(pkg, _)| String::from("fil_actor_") + pkg).collect::<Vec<String>>();

    let manifest_path =
        Path::new(&std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR unset"))
            .join("Cargo.toml");
    println!("cargo:warning=manifest_path={:?}", &manifest_path);

    // Cargo build command for all actors at once.
    let mut cmd = Command::new(&cargo);
    cmd.arg("build")
        .args(packages.iter().map(|pkg| "-p=".to_owned() + pkg))
        .arg("--target=wasm32-unknown-unknown")
        .arg("--profile=wasm")
        .arg("--locked")
        .arg("--manifest-path=".to_owned() + manifest_path.to_str().unwrap())
        .env("RUSTFLAGS", "-Ctarget-feature=+crt-static -Clink-arg=--export-table")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // We are supposed to only generate artifacts under OUT_DIR,
        // so set OUT_DIR as the target directory for this build.
        .env("CARGO_TARGET_DIR", &out_dir)
        // As we are being called inside a build-script, this env variable is set. However, we set
        // our own `RUSTFLAGS` and thus, we need to remove this. Otherwise cargo favors this
        // env variable.
        .env_remove("CARGO_ENCODED_RUSTFLAGS");

    // Print out the command line we're about to run.
    println!("cargo:warning=cmd={:?}", &cmd);

    // Launch the command.
    let mut child = cmd.spawn().expect("failed to launch cargo build");

    // Pipe the output as cargo warnings. Unfortunately this is the only way to
    // get cargo build to print the output.
    let stdout = child.stdout.take().expect("no stdout");
    let stderr = child.stderr.take().expect("no stderr");
    let j1 = thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            println!("cargo:warning={:?}", line.unwrap());
        }
    });
    let j2 = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            println!("cargo:warning={:?}", line.unwrap());
        }
    });

    j1.join().unwrap();
    j2.join().unwrap();

    let dst = Path::new(&out_dir).join("bundle.car");
    let mut bundler = Bundler::new(&dst);
    for (pkg, id) in ACTORS {
        let bytecode_path = Path::new(&out_dir)
            .join("wasm32-unknown-unknown/wasm")
            .join(format!("fil_actor_{}.wasm", pkg));

        // This actor version uses forced CIDs.
        let forced_cid = {
            let identity = FORCED_CID_PREFIX.to_owned() + id;
            Cid::new_v1(IPLD_RAW, Multihash::wrap(0, identity.as_bytes())?)
        };

        let cid = bundler
            .add_from_file((*id).try_into().unwrap(), Some(&forced_cid), &bytecode_path)
            .unwrap_or_else(|err| {
                panic!("failed to add file {:?} to bundle for actor {}: {}", bytecode_path, id, err)
            });
        println!("cargo:warning=added actor {} to bundle with CID {}", id, cid);
    }
    bundler.finish().expect("failed to finish bundle");

    println!("cargo:warning=bundle={}", dst.display());

    Ok(())
}
