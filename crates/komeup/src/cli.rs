use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "komeup")]
#[command(version)]
#[command(about = "Kome toolchain installer")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    Install {
        #[arg(long, default_value = "stable")]
        channel: String,

        #[arg(long)]
        force: bool,
    },

    Update {
        #[arg(long, default_value = "stable")]
        channel: String,
    },

    Status,

    Doctor,

    Default {
        toolchain: String,
    },

    Uninstall {
        #[arg(long)]
        yes: bool,
    },
}
