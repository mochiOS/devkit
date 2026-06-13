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