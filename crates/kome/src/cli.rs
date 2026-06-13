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
    Keygen(KeygenArgs),
    Sign(SignArgs),
    Verify(VerifyArgs),
}

#[derive(Debug, Args)]
pub struct KeygenArgs {
    #[arg(long, default_value = "application.key")]
    pub private_key: PathBuf,

    #[arg(long, default_value = "application.pub")]
    pub public_key: PathBuf,
}

#[derive(Debug, Args)]
pub struct VerifyArgs {
    pub package: Option<PathBuf>,

    #[arg(long)]
    pub pubkey: Option<PathBuf>,

    #[arg(short = 'l', long)]
    pub local: bool,

    #[arg(long)]
    pub api_base: Option<String>,
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
}

#[derive(Debug, Args)]
pub struct SignArgs {
    pub package: Option<PathBuf>,

    #[arg(long, default_value = ".")]
    pub project: PathBuf,

    #[arg(long, default_value = "application.key")]
    pub key: PathBuf,

    #[arg(long, default_value = "application")]
    pub key_id: String,
}