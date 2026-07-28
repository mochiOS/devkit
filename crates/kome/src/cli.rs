use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "kome")]
#[command(about = "Kome project manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    New(NewArgs),
    Build(BuildArgs),
    Pack(PackArgs),
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
    Certificate {
        #[command(subcommand)]
        command: CertificateCommand,
    },
    Keygen(KeygenArgs),
    Sign(SignArgs),
    Verify(VerifyArgs),
}

#[derive(Debug, Subcommand)]
pub enum KeyCommand {
    Generate(KeygenArgs),
}

#[derive(Debug, Subcommand)]
pub enum CertificateCommand {
    Obtain(CertificateObtainArgs),
}

#[derive(Debug, Args)]
pub struct KeygenArgs {
    #[arg(long, default_value = "keys/application.key")]
    pub private_key: PathBuf,

    #[arg(long, default_value = "keys/application.pub")]
    pub public_key: PathBuf,
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
    pub package: Option<PathBuf>,

    #[arg(long, default_value = ".")]
    pub project: PathBuf,

    #[arg(long)]
    pub pubkey: Option<PathBuf>,

    #[arg(short = 'l', long)]
    pub local: bool,

    #[arg(long)]
    pub api_base: Option<String>,

    #[arg(long, alias = "issuer-public-key")]
    pub root_public_key: Option<PathBuf>,

    #[arg(long)]
    pub unix_time: Option<u64>,

    #[arg(long)]
    pub legacy: bool,
}

#[derive(Debug, Args)]
pub struct NewArgs {
    pub name: String,

    #[arg(long)]
    pub id: Option<String>,

    #[arg(long, default_value = "mochi from around the world")]
    pub developer: String,
}

#[derive(Debug, Args)]
pub struct BuildArgs {
    #[arg(default_value = ".")]
    pub project_dir: PathBuf,
}

#[derive(Debug, Args)]
pub struct PackArgs {
    #[arg(default_value = ".")]
    pub project_dir: PathBuf,

    #[arg(short, long)]
    pub output: Option<PathBuf>,

    #[arg(long)]
    pub release: bool,

    #[arg(long)]
    pub force: bool,

    #[arg(long)]
    pub legacy: bool,
}

#[derive(Debug, Args)]
pub struct SignArgs {
    pub package: Option<PathBuf>,

    #[arg(long, default_value = ".")]
    pub project: PathBuf,

    #[arg(long, default_value = "keys/application.key")]
    pub key: PathBuf,

    #[arg(long, default_value = "application")]
    pub key_id: String,

    #[arg(long, default_value = "keys/developer.cert")]
    pub certificate: PathBuf,

    #[arg(short, long)]
    pub output: Option<PathBuf>,

    #[arg(long)]
    pub unix_time: Option<u64>,

    #[arg(long)]
    pub legacy: bool,
}

#[derive(Debug, Args)]
pub struct CertificateObtainArgs {
    #[arg(long)]
    pub developer: String,

    #[arg(long, default_value = "keys/application.pub")]
    pub public_key: PathBuf,

    #[arg(long)]
    pub package: Option<PathBuf>,

    #[arg(long, default_value = "keys/developer.cert")]
    pub output: PathBuf,

    #[arg(long, default_value = "https://api.mochios.org/v1")]
    pub api_base: String,

    #[arg(long)]
    pub bearer_token: Option<String>,

    #[arg(long, default_value = ".")]
    pub project: PathBuf,
}
