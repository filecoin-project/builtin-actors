use clap::Parser;
use std::io::Write;

use fil_builtin_actors_bundle::BUNDLE_CAR;

#[derive(Parser)]
#[clap(name = env!("CARGO_PKG_NAME"))]
#[clap(version = env!("CARGO_PKG_VERSION"))]
#[clap(about = "Writes a CAR file containing Wasm bytecode for Filecoin actors.", long_about = None)]
struct Cli {
    /// The output car path. Defaults to STDOUT.
    #[clap(short, long, required = false)]
    output: Option<String>,
}

fn main() -> Result<(), std::io::Error> {
    let cli = Cli::parse();
    match cli.output {
        Some(path) => std::fs::write(path, BUNDLE_CAR),
        None => std::io::stdout().write_all(BUNDLE_CAR),
    }
}
