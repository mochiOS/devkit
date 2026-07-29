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
}

#[derive(Debug, Subcommand)]
pub enum KeyCommand {
    Generate(KeygenArgs),
}

#[derive(Debug, Subcommand)]
pub enum CertificateCommand {
    Issue(CertificateIssueArgs),
    Inspect(CertificateInspectArgs),
    Obtain(CertificateObtainArgs),
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
pub struct CertificateIssueArgs {
    #[arg(long)]
    pub root_key: Option<PathBuf>,

    #[arg(long)]
    pub issuer_key: Option<PathBuf>,

    #[arg(long)]
    pub developer_key: Option<PathBuf>,

    #[arg(long)]
    pub subject_public_key: Option<PathBuf>,

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
pub struct CertificateObtainArgs {
    #[arg(long)]
    pub developer: String,

    #[arg(long)]
    pub public_key: PathBuf,

    #[arg(long)]
    pub package: PathBuf,

    #[arg(long)]
    pub output: PathBuf,

    #[arg(long, default_value = "https://ca.mochios.org/v1")]
    pub api_base: String,

    #[arg(long)]
    pub bearer_token: Option<String>,

    #[arg(long)]
    pub idempotency_key: Option<String>,
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

    #[arg(long)]
    pub unix_time: Option<u64>,

    #[arg(long)]
    pub replace_signature: bool,
}

#[derive(Debug, Args)]
pub struct PackageVerifyArgs {
    pub package: PathBuf,

    #[arg(long, alias = "issuer-public-key")]
    pub root_public_key: PathBuf,

    #[arg(long)]
    pub unix_time: u64,
}
