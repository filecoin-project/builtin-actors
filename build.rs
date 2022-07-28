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

const WASM_FEATURES: &[&str] = &["+bulk-memory", "+crt-static"];

/// Default Cargo features to activate during the build.
const DEFAULT_CARGO_FEATURES: &[&str] = &["fil-actor"];

/// Extra Cargo-level features to enable per network.
const EXTRA_CARGO_FEATURES: &[(&str, &[&str])] = &[
    ("wallaby", &["m2-native"]),
];

const NETWORK_ENV: &str = "BUILD_FIL_NETWORK";

/// Returns the configured network name, checking both the environment and feature flags.
fn network_name() -> String {
    let env_network = std::env::var_os(NETWORK_ENV);

    let feat_network = if cfg!(feature = "mainnet") {
        Some("mainnet")
    } else if cfg!(feature = "caterpillarnet") {
        Some("caterpillarnet")
    } else if cfg!(feature = "butterflynet") {
        Some("butterflynet")
    } else if cfg!(feature = "calibrationnet") {
        Some("calibrationnet")
    } else if cfg!(feature = "devnet") {
        Some("devnet")
    } else if cfg!(feature = "devnet-m2-native") {
        Some("devnet-m2-native")
    } else if cfg!(feature = "testing") {
        Some("testing")
    } else if cfg!(feature = "testing-fake-proofs") {
        Some("testing-fake-proofs")
    } else {
        None
    };

    // Make sure they match if they're both set. Otherwise, pick the one
    // that's set, or fallback on "mainnet".
    match (feat_network, &env_network) {
        (Some(from_feature), Some(from_env)) => {
            assert_eq!(from_feature, from_env, "different target network configured via the features than via the {} environment variable", NETWORK_ENV);
            from_feature
        }
        (Some(net), None) => net,
        (None, Some(net)) => net.to_str().expect("network name not utf8"),
        (None, None) => "mainnet",
    }.to_owned()
}

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

    // Determine the network name.
    let network_name = network_name();
    println!("cargo:warning=network name: {}", network_name);

    // Make sure we re-build if the network name changes.
    println!("cargo:rerun-if-env-changed={}", NETWORK_ENV);

    // Rerun if the source, dependencies, build options, build script _or_ actors have changed. We
    // need to check if the actors have changed because otherwise, when building in a workspace, we
    // won't re-run the build script and therefore won't re-compile them.
    //
    // This _isn't_ an issue when building as a dependency fetched from crates.io (because the crate
    // is immutable).
    for file in ["actors", "Cargo.toml", "Cargo.lock", "src", "build.rs"] {
        println!("cargo:rerun-if-changed={}", file);
    }

    let rustflags =
        WASM_FEATURES.iter().flat_map(|flag| ["-Ctarget-feature=", *flag, " "]).collect::<String>()
            + "-Clink-arg=--export-table";

    // Compute Cargo features to apply.
    let features = {
        let extra = EXTRA_CARGO_FEATURES.iter().find(|(k, _)| k == &network_name).map(|f| f.1).unwrap_or_default();
        [DEFAULT_CARGO_FEATURES, extra].concat()
    };

    // Cargo build command for all actors at once.
    let mut cmd = Command::new(&cargo);
    cmd.arg("build")
        .args(packages.iter().map(|pkg| "-p=".to_owned() + pkg))
        .arg("--target=wasm32-unknown-unknown")
        .arg("--profile=wasm")
        .arg("--locked")
        .arg("--features=".to_owned() + &features.join(","))
        .arg("--manifest-path=".to_owned() + manifest_path.to_str().unwrap())
        .env("RUSTFLAGS", rustflags)
        .env(NETWORK_ENV, network_name)
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

    let result = child.wait().expect("failed to wait for build to finish");
    if !result.success() {
        return Err("actor build failed".into());
    }

    let dst = Path::new(&out_dir).join("bundle.car");
    let mut bundler = Bundler::new(&dst);
    for (pkg, id) in ACTORS {
        let bytecode_path = Path::new(&out_dir)
            .join("wasm32-unknown-unknown/wasm")
            .join(format!("fil_actor_{}.wasm", pkg));

        // This actor version doesn't force synthetic CIDs; it uses genuine
        // content-addressed CIDs.
        let forced_cid = None;

        let cid = bundler.add_from_file(id, forced_cid, &bytecode_path).unwrap_or_else(|err| {
            panic!("failed to add file {:?} to bundle for actor {}: {}", bytecode_path, id, err)
        });
        println!("cargo:warning=added actor {} to bundle with CID {}", id, cid);
    }
    bundler.finish().expect("failed to finish bundle");

    println!("cargo:warning=bundle={}", dst.display());

    Ok(())
}
