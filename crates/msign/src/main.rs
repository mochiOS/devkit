mod cli;
mod commands;
mod crypto;
mod elf_signature;

#[cfg(target_os = "mochios")]
getrandom02::register_custom_getrandom!(mochios_posix_client::random::getrandom02::fill);

use anyhow::Result;
use clap::Parser;

use crate::cli::{
    CertificateCommand, Cli, Command, ElfCommand, IdentityCommand, KeyCommand, PackageCommand,
};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Package {
            command: PackageCommand::Requirements(args),
        } => commands::mpkg::requirements(args),
        Command::Elf {
            command: ElfCommand::Sign(args),
        } => commands::elf::sign(args),
        Command::Elf {
            command: ElfCommand::Verify(args),
        } => commands::elf::verify(args),
        Command::Key {
            command: KeyCommand::Generate(args),
        } => commands::keygen::run(args),
        Command::Certificate {
            command: CertificateCommand::Issue(args),
        } => commands::certificate::issue(args),
        Command::Certificate {
            command: CertificateCommand::Inspect(args),
        } => commands::certificate::inspect(args),
        #[cfg(feature = "developer-ca")]
        Command::Certificate {
            command: CertificateCommand::Obtain(args),
        } => commands::certificate::obtain(args),
        Command::Package {
            command: PackageCommand::Sign(args),
        } => commands::mpkg::sign(args),
        Command::Package {
            command: PackageCommand::SignAuto(args),
        } => commands::identity::sign_auto(args),
        Command::Package {
            command: PackageCommand::Verify(args),
        } => commands::mpkg::verify(args),
        Command::Package {
            command: PackageCommand::BindBuiltIn(args),
        } => commands::builtin_bind::bind(args),
        Command::Package {
            command: PackageCommand::BuiltInRecord(args),
        } => commands::builtin::record(args),
        Command::Identity {
            command: IdentityCommand::Import(args),
        } => commands::identity::import(args),
        Command::Identity {
            command: IdentityCommand::List,
        } => commands::identity::list(),
    }
}
