use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "mpack")]
#[command(about = "mochiOS package builder")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Pack(PackArgs),
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