mod cli;
mod fs;
mod mock_pkg;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::MockPkg(args) => mock_pkg::run(args),
    }
}
