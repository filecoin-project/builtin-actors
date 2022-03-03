use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

use anyhow::Context;

const TARGET: &str = "wasm32-unknown-unknown";
const PROFILE: &str = "wasm";

pub fn build() -> anyhow::Result<()> {
    // Cargo executable location.
    let cargo = std::env::var_os("CARGO").context("no CARGO env var")?;

    let out_dir: PathBuf = std::env::var_os("OUT_DIR")
        .context("no OUT_DIR set")?
        .into();
    let wasm_target_dir = out_dir.join("wasm-target");

    let mut manifest_path: PathBuf = std::env::var_os("CARGO_MANIFEST_DIR")
        .context("CARGO_MANIFEST_DIR unset")?
        .into();
    manifest_path.push("Cargo.toml");

    let crate_name =
        std::env::var_os("CARGO_PKG_NAME").context("failed to get CARGO_CRATE_NAME")?;

    // Cargo build command for all actors at once.
    let mut cmd = Command::new(&cargo);
    cmd.arg("build")
        .arg(format!("--target={}", TARGET))
        .arg(format!("--profile={}", PROFILE))
        .arg(
            ["--manifest-path=".as_ref(), manifest_path.as_ref()]
                .into_iter()
                .collect::<OsString>(),
        )
        .env(
            "RUSTFLAGS",
            "-Ctarget-feature=+crt-static -Clink-arg=--export-table",
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // We are supposed to only generate artifacts under OUT_DIR,
        // so set OUT_DIR as the target directory for this build.
        .env("CARGO_TARGET_DIR", &wasm_target_dir)
        // As we are being called inside a build-script, this env variable is set. However, we set
        // our own `RUSTFLAGS` and thus, we need to remove this. Otherwise cargo favors this
        // env variable.
        .env_remove("CARGO_ENCODED_RUSTFLAGS");

    // Launch the command.
    let mut child = cmd.spawn().context("failed to launch cargo build")?;

    // Pipe the output as cargo warnings. Unfortunately this is the only way to
    // get cargo build to print the output.
    let stdout = child.stdout.take().context("no stdout")?;
    let stderr = child.stderr.take().context("no stderr")?;
    let j1 = thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            println!("cargo:warning={}", line.unwrap());
        }
    });
    let j2 = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            println!("cargo:warning={}", line.unwrap());
        }
    });

    j1.join().unwrap();
    j2.join().unwrap();
    child.wait()?;

    let wasm_output_path: PathBuf = [
        &*wasm_target_dir,
        TARGET.as_ref(),
        PROFILE.as_ref(),
        [&*crate_name, ".wasm".as_ref()]
            .into_iter()
            .collect::<OsString>()
            .as_ref(),
    ]
    .into_iter()
    .collect();

    println!(
        "cargo:rustc-env=WASM_BINARY={}",
        wasm_output_path.display().to_string().escape_default()
    );

    Ok(())
}
