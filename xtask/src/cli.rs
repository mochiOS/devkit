use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "xtask")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    MockPkg(MockPkgArgs),
}

#[derive(Debug, Args)]
pub struct MockPkgArgs {
    #[arg(short, long, default_value = "exampleApplication.pkg")]
    pub output: PathBuf,

    #[arg(long, default_value = "ExampleApp")]
    pub name: String,

    #[arg(long, default_value = "org.mochios.example")]
    pub bundle_id: String,

    #[arg(long, default_value = "0.1.0")]
    pub version: String,

    #[arg(long, default_value = "KonoyoniSonzaisuruSubetenoOmochi")]
    pub developer: String,
}