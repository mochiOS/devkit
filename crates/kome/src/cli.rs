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
    Sign(SignArgs),
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
    #[arg(default_value = ".")]
    pub project_dir: PathBuf,

    #[arg(long)]
    pub key: PathBuf,

    #[arg(long)]
    pub key_id: String,

    #[arg(short, long)]
    pub package: Option<PathBuf>,
}