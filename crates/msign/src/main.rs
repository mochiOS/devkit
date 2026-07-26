mod cli;
mod commands;
mod crypto;
mod package;
mod signature;

use anyhow::Result;
use clap::Parser;

use crate::cli::{CertificateCommand, Cli, Command, KeyCommand, PackageCommand};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Key {
            command: KeyCommand::Generate(args),
        } => commands::keygen::run(args),
        Command::Certificate {
            command: CertificateCommand::Issue(args),
        } => commands::certificate::issue(args),
        Command::Certificate {
            command: CertificateCommand::Inspect(args),
        } => commands::certificate::inspect(args),
        Command::Package {
            command: PackageCommand::Sign(args),
        } => commands::mpkg::sign(args),
        Command::Package {
            command: PackageCommand::Verify(args),
        } => commands::mpkg::verify(args),
        Command::Keygen(args) => commands::keygen::run(args),
        Command::Sign(args) => commands::sign::run(args),
        Command::Verify(args) => commands::verify::run(args),
    }
}
