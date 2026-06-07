mod cli;
mod commands;
mod crypto;
mod package;
mod signature;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Keygen(args) => commands::keygen::run(args),
        Command::Sign(args) => commands::sign::run(args),
        Command::Verify(args) => commands::verify::run(args),
    }
}