mod app_files;
mod cli;
mod manifest;
mod package;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Pack(args) => package::pack(args),
    }
}