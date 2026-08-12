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
    Create(CreateArgs),
    Linux(LinuxArgs),
}

#[derive(Debug, Args)]
#[group(required = true, multiple = false)]
pub struct LinuxSource {
    #[arg(long)]
    pub apt_package: Option<String>,

    #[arg(long)]
    pub linux_binary: Option<PathBuf>,

    #[arg(long)]
    pub rootfs: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct LinuxArgs {
    #[arg(long)]
    pub bundle_id: String,

    #[arg(long)]
    pub name: String,

    #[arg(long)]
    pub version: String,

    #[arg(long)]
    pub vendor: String,

    #[arg(long)]
    pub entrypoint: String,

    #[command(flatten)]
    pub source: LinuxSource,

    #[arg(long, default_value = "amd64")]
    pub architecture: String,

    #[arg(long = "writable-path")]
    pub writable_paths: Vec<String>,

    #[arg(long = "portal-read")]
    pub portal_read_paths: Vec<String>,

    #[arg(long = "portal-write")]
    pub portal_write_paths: Vec<String>,

    #[arg(long)]
    pub icon: Option<PathBuf>,

    #[arg(short, long)]
    pub output: PathBuf,

    #[arg(long)]
    pub force: bool,
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
pub struct CreateArgs {
    #[arg(long)]
    pub manifest: PathBuf,

    #[arg(long)]
    pub payload: PathBuf,

    #[arg(short, long)]
    pub output: PathBuf,

    #[arg(long)]
    pub force: bool,
}
