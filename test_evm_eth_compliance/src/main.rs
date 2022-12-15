mod cmd;
mod common;
mod statetest;

use cmd::Error;
use structopt::StructOpt;

pub fn main() -> Result<(), Error> {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    let cmd = cmd::MainCmd::from_args();
    cmd.run()
}
