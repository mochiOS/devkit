mod cli;
mod commands;
mod manifest;
mod project;

use anyhow::Result;
use clap::Parser;

use crate::cli::{CertificateCommand, Cli, Command, KeyCommand};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::New(args) => commands::new::run(args),
        Command::Build(args) => commands::build::run(args),
        Command::Pack(args) => commands::pack::run(args),
        Command::Key {
            command: KeyCommand::Generate(args),
        } => commands::keygen::run(args),
        Command::Certificate {
            command: CertificateCommand::Obtain(args),
        } => commands::certificate::obtain(args),
        Command::Sign(args) => commands::sign::run(args),
        Command::Keygen(args) => commands::keygen::run(args),
        Command::Verify(args) => commands::verify::run(args),
    }
}
