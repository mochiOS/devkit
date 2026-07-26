use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "msign")]
#[command(about = "mochiOS package signing tool")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    Certificate {
        #[command(subcommand)]
        command: CertificateCommand,
    },
    Package {
        #[command(subcommand)]
        command: PackageCommand,
    },
    /// Legacy alias used by Kome.
    Keygen(KeygenArgs),
    /// Legacy .pkg signing command used by Kome.
    Sign(SignArgs),
    /// Legacy .pkg verification command used by Kome.
    Verify(VerifyArgs),
}

#[derive(Debug, Subcommand)]
pub enum KeyCommand {
    Generate(KeygenArgs),
}

#[derive(Debug, Subcommand)]
pub enum CertificateCommand {
    Issue(CertificateIssueArgs),
    Inspect(CertificateInspectArgs),
}

#[derive(Debug, Subcommand)]
pub enum PackageCommand {
    Sign(PackageSignArgs),
    Verify(PackageVerifyArgs),
}

#[derive(Debug, Args)]
pub struct KeygenArgs {
    #[arg(long, default_value = "application.key")]
    pub private_key: PathBuf,

    #[arg(long, default_value = "application.pub")]
    pub public_key: PathBuf,
}

#[derive(Debug, Args)]
pub struct SignArgs {
    pub package: PathBuf,

    #[arg(long)]
    pub key: PathBuf,

    #[arg(long)]
    pub key_id: String,

    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
    pub package: PathBuf,

    #[arg(long)]
    pub pubkey: Option<PathBuf>,

    #[arg(short = 'l', long)]
    pub local: bool,

    #[arg(long)]
    pub api_base: Option<String>,
}

#[derive(Debug, Args)]
pub struct CertificateIssueArgs {
    #[arg(long)]
    pub root_key: PathBuf,

    #[arg(long)]
    pub developer_key: PathBuf,

    #[arg(long)]
    pub output: PathBuf,

    #[arg(long)]
    pub serial: u64,

    #[arg(long)]
    pub developer_id: String,

    #[arg(long)]
    pub not_before: u64,

    #[arg(long)]
    pub not_after: u64,

    #[arg(long = "scope", required = true)]
    pub scopes: Vec<String>,

    #[arg(long = "capability")]
    pub capabilities: Vec<String>,
}

#[derive(Debug, Args)]
pub struct CertificateInspectArgs {
    pub certificate: PathBuf,
}

#[derive(Debug, Args)]
pub struct PackageSignArgs {
    pub package: PathBuf,

    #[arg(long)]
    pub certificate: PathBuf,

    #[arg(long)]
    pub key: PathBuf,

    #[arg(short, long)]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct PackageVerifyArgs {
    pub package: PathBuf,

    #[arg(long)]
    pub root_public_key: PathBuf,

    #[arg(long)]
    pub unix_time: u64,
}
