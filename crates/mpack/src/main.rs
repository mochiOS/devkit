mod app_files;
mod cli;
mod manifest;
mod mpkg;
mod package;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Pack(args) => package::pack(args),
        Command::Create(args) => mpkg::create(args),
    }
}
