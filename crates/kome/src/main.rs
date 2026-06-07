mod cli;
mod commands;
mod manifest;
mod project;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::New(args) => commands::new::run(args),
        Command::Build(args) => commands::build::run(args),
        Command::Pack(args) => commands::pack::run(args),
        Command::Sign(args) => commands::sign::run(args),
    }
}