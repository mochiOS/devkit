use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "kome")]
#[command(about = "mochiOS project manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    New(NewArgs),
    Build(BuildArgs),
}

#[derive(Debug, Args)]
pub struct NewArgs {
    pub name: String,

    #[arg(long)]
    pub id: Option<String>,

    #[arg(long, default_value = "tas0dev")]
    pub developer: String,
}

#[derive(Debug, Args)]
pub struct BuildArgs {
    #[arg(default_value = ".")]
    pub project_dir: PathBuf,
}