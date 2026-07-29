use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::auth::DEFAULT_ACCOUNTS_API_BASE;

#[derive(Debug, Parser)]
#[command(name = "kome")]
#[command(about = "Kome project manager")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Authenticate Kome through the mochiOS Account device flow.
    Login(LoginArgs),
    /// Display the active Kome CLI account session.
    Account(AccountArgs),
    /// Revoke and remove the active Kome CLI account session.
    Logout(LogoutArgs),
    /// List or select Developers available to the authenticated Account.
    Developer {
        #[command(subcommand)]
        command: DeveloperCommand,
    },
    /// Create a new Kome application project.
    New(NewArgs),
    /// Build the current Kome project.
    Build(BuildArgs),
    /// Generate an unsigned MPKG v1 package.
    Pack(PackArgs),
    /// Generate or validate the project's Ed25519 application key pair.
    Keygen(KeygenArgs),
    /// Build, package, obtain a certificate, sign, and verify the project.
    Sign(SignArgs),
    /// Verify a signed MPKG with its issuer public key.
    Verify(VerifyArgs),
}

#[derive(Debug, Subcommand)]
pub enum DeveloperCommand {
    /// List Developers available through DeveloperCA.
    List(DeveloperListArgs),
    /// Select the default Developer used by Kome signing.
    Use(DeveloperUseArgs),
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    #[arg(
        long,
        env = "KOME_ACCOUNTS_API_BASE",
        default_value = DEFAULT_ACCOUNTS_API_BASE
    )]
    pub accounts_api_base: String,

    /// Print the verification URL without opening a browser.
    #[arg(long)]
    pub no_browser: bool,
}

#[derive(Debug, Args)]
pub struct AccountArgs {
    #[arg(
        long,
        env = "KOME_ACCOUNTS_API_BASE",
        default_value = DEFAULT_ACCOUNTS_API_BASE
    )]
    pub accounts_api_base: String,
}

pub type LogoutArgs = AccountArgs;

#[derive(Debug, Args)]
pub struct DeveloperListArgs {
    #[arg(
        long,
        env = "KOME_ACCOUNTS_API_BASE",
        default_value = DEFAULT_ACCOUNTS_API_BASE
    )]
    pub accounts_api_base: String,

    #[arg(
        long,
        env = "KOME_DEVELOPER_CA_API_BASE",
        default_value = crate::certificate_client::DEFAULT_DEVELOPER_CA_API_BASE
    )]
    pub developer_ca_api_base: String,
}

#[derive(Debug, Args)]
pub struct DeveloperUseArgs {
    pub developer_id: String,

    #[command(flatten)]
    pub api: DeveloperListArgs,
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

    #[arg(long, alias = "issuer-public-key")]
    pub root_public_key: Option<PathBuf>,

    #[arg(long)]
    pub unix_time: Option<u64>,
}

#[derive(Debug, Args)]
pub struct NewArgs {
    pub name: String,

    #[arg(long)]
    pub id: Option<String>,

    #[arg(long, default_value = "mochi from around the world")]
    pub vendor: String,
}

#[derive(Debug, Args)]
pub struct BuildArgs {
    #[arg(default_value = ".")]
    pub project_dir: PathBuf,

    #[arg(long)]
    pub release: bool,
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
    /// Unsigned MPKG input; defaults to dist/<name>-unsigned.mpkg.
    pub package: Option<PathBuf>,

    #[arg(long, default_value = ".")]
    pub project: PathBuf,

    #[arg(long, default_value = "keys/application.key")]
    pub key: PathBuf,

    #[arg(long, default_value = "keys/application.pub")]
    pub public_key: PathBuf,

    #[arg(long, default_value = "keys/developer.cert")]
    pub certificate: PathBuf,

    #[arg(long, default_value = "keys/developer.issuer.pub")]
    pub issuer_public_key: PathBuf,

    #[arg(short, long)]
    /// Signed MPKG output; defaults to dist/<name>.mpkg.
    pub output: Option<PathBuf>,

    #[arg(long)]
    pub unix_time: Option<u64>,

    #[arg(long)]
    pub release: bool,

    /// Start Account login when no Kome CLI credential is available.
    #[arg(long)]
    pub login: bool,

    #[arg(
        long,
        env = "KOME_ACCOUNTS_API_BASE",
        default_value = DEFAULT_ACCOUNTS_API_BASE
    )]
    pub accounts_api_base: String,

    #[arg(
        long,
        env = "KOME_DEVELOPER_CA_API_BASE",
        default_value = crate::certificate_client::DEFAULT_DEVELOPER_CA_API_BASE
    )]
    pub developer_ca_api_base: String,
}
