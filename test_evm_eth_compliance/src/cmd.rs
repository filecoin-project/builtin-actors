use crate::statetest;
use structopt::{clap::AppSettings, StructOpt};

use thiserror::Error as ThisError;

#[derive(Debug, ThisError)]
pub enum Error {
    #[error("Statetest: {0}")]
    Statetest(statetest::Error),
    #[error("Generic system error")]
    SystemError,
}

#[derive(StructOpt, Debug)]
#[structopt(setting = AppSettings::InferSubcommands)]
#[allow(clippy::large_enum_variant)]
pub enum MainCmd {
    Statetest(statetest::Cmd),
    Dummytest,
}

impl MainCmd {
    pub fn run(&self) -> Result<(), Error> {
        match self {
            Self::Statetest(cmd) => cmd.run().map_err(Error::Statetest),
            _ => Ok(()),
        }
    }
}
