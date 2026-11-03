use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "komeup")]
#[command(version)]
#[command(about = "Kome toolchain and mochiOS SDK manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Manage separately versioned mochiOS C/C++ SDKs.
    Sdk {
        #[command(subcommand)]
        command: crate::sdk::SdkCommand,
    },

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
