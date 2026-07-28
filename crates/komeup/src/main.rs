mod cli;
mod commands;
mod config;
mod github;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Install { channel, force } => {
            commands::install::run(&channel, force)?;
        }
        Command::Update { channel } => {
            commands::install::run(&channel, true)?;
        }
        Command::Status => {
            commands::status::run()?;
        }
        Command::Doctor => {
            commands::doctor::run()?;
        }
        Command::Default { toolchain } => {
            commands::set_default::run(&toolchain)?;
        }
        Command::Uninstall { yes } => {
            commands::uninstall::run(yes)?;
        }
    }

    Ok(())
}
