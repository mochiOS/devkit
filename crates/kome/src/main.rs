mod auth;
mod certificate_client;
mod cli;
mod commands;
mod credential;
mod developer_selection;
mod manifest;
mod preferences;
mod project;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Command, DeveloperCommand};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Login(args) => commands::login::run(args),
        Command::Account(args) => commands::account::run(args),
        Command::Logout(args) => commands::logout::run(args),
        Command::Developer {
            command: DeveloperCommand::List(args),
        } => commands::developer::list(args),
        Command::Developer {
            command: DeveloperCommand::Use(args),
        } => commands::developer::use_developer(args),
        Command::New(args) => commands::new::run(args),
        Command::Build(args) => commands::build::run(args),
        Command::Pack(args) => commands::pack::run(args),
        Command::Sign(args) => commands::sign::run(args),
        Command::Keygen(args) => commands::keygen::run(args),
        Command::Verify(args) => commands::verify::run(args),
    }
}
