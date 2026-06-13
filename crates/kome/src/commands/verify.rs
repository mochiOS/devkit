use std::{env, path::PathBuf, process::Command};

use anyhow::{bail, Context, Result};

use crate::{cli::VerifyArgs, project};

pub fn run(args: VerifyArgs) -> Result<()> {
    let package = match args.package {
        Some(package) => package,
        None => default_package_path()?,
    };

    let mut command = Command::new("msign");

    command.arg("verify").arg(&package);

    if let Some(pubkey) = args.pubkey {
        command.arg("--pubkey").arg(pubkey);
    }

    if args.local {
        command.arg("--local");
    }

    if let Some(api_base) = args.api_base {
        command.arg("--api-base").arg(api_base);
    }

    let status = command
        .status()
        .context("failed to execute msign. is msign installed?")?;

    if !status.success() {
        bail!("msign verify failed");
    }

    Ok(())
}

fn default_package_path() -> Result<PathBuf> {
    let project_dir = env::current_dir().context("failed to get current directory")?;
    let manifest = project::read_manifest(&project_dir)?;

    Ok(project_dir
        .join("target/package")
        .join(format!("{}.pkg", manifest.package.name)))
}