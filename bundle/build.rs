use fil_actor_bundler::Bundler;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

const ACTORS: &[&'static str] = &[
    "account", "cron", "market", "miner", "multisig", "paych", "reward", "system", "verifreg",
    "power", "init",
];

fn main() {
    // Cargo executable location.
    let cargo = std::env::var_os("CARGO").expect("no CARGO env var");

    // Compute the package names.
    let packages = ACTORS
        .iter()
        .map(|actor| String::from("fvm_actor_") + actor)
        .collect::<Vec<String>>();

    // Cargo build command for all actors at once.
    let mut cmd = Command::new(&cargo);
    cmd.arg("build")
        .args(packages.iter().map(|pkg| String::from("-p=") + pkg))
        .arg("--target=wasm32-unknown-unknown")
        .arg("--profile=wasm")
        .env(
            "RUSTFLAGS",
            "-Ctarget-feature=+crt-static -Clink-arg=--export-table",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Unset the `CARGO_TARGET_DIR` to prevent a cargo deadlock (cargo locks a target dir
        // exclusive). The runner project is created in `CARGO_TARGET_DIR` and executing it will
        // create a sub target directory inside of `CARGO_TARGET_DIR`.
        .env_remove("CARGO_TARGET_DIR")
        // As we are being called inside a build-script, this env variable is set. However, we set
        // our own `RUSTFLAGS` and thus, we need to remove this. Otherwise cargo favors this
        // env variable.
        .env_remove("CARGO_ENCODED_RUSTFLAGS");

    // Print out the command line we're about to run.
    println!("cargo:warning={:?}", &cmd);

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

    let target_dir = locate_target_dir().expect("no target directory located");
    println!(
        "cargo:warning=target directory located at: {:?}",
        &target_dir
    );

    let out_dir = std::env::var_os("OUT_DIR").expect("no OUT_DIR env var");
    let dst = Path::new(&out_dir).join("bundle.car");
    let mut bundler = Bundler::new(&dst);
    for act in ACTORS {
        let bytecode_path = target_dir
            .join("wasm32-unknown-unknown/wasm")
            .join(format!("fvm_actor_{}.wasm", act));
        let cid = bundler
            .add_from_file((*act).to_owned(), None, &bytecode_path)
            .unwrap_or_else(|err| {
                panic!(
                    "failed to add file {:?} to bundle for actor {}: {}",
                    bytecode_path, act, err
                )
            });
        println!(
            "cargo:warning=added actor {} to bundle with CID {}",
            act, cid
        );
    }
    bundler.finish().expect("failed to finish bundle");

    println!("cargo:warning=bundle written to: {}", dst.to_str().unwrap());
}

/// Locates the workspce target directory by walking up from the OUT_DIR.
fn locate_target_dir() -> Option<PathBuf> {
    let mut out_dir = std::env::var_os("OUT_DIR")
        .map(|dir| PathBuf::from(dir))
        .expect("no OUT_DIR env variable");

    loop {
        if out_dir.ends_with("target") {
            return Some(out_dir);
        }
        if !out_dir.pop() {
            return None;
        }
    }
}
