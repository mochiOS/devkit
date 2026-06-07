use std::{path::PathBuf, process::Command};

use anyhow::{bail, Context, Result};

use crate::{cli::VerifyArgs, project};

pub fn run(args: VerifyArgs) -> Result<()> {
    let package = match args.package {
        Some(package) => package,
        None => default_package_path(&args.project_dir)?,
    };

    let mut command = Command::new("msign");

    command.arg("verify").arg(&package);

    if let Some(pubkey) = args.pubkey {
        command.arg("--pubkey").arg(pubkey);
    }

    let status = command
        .status()
        .context("failed to execute msign. is msign installed?")?;

    if !status.success() {
        bail!("msign verify failed");
    }

    Ok(())
}

fn default_package_path(project_dir: &std::path::Path) -> Result<PathBuf> {
    let manifest = project::read_manifest(project_dir)?;

    Ok(project_dir
        .join("target/package")
        .join(format!("{}.pkg", manifest.package.name)))
}