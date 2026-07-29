use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::{cli::VerifyArgs, commands::sign::default_signed_package_path};

pub fn run(args: VerifyArgs) -> Result<()> {
    let package = match args.package {
        Some(package) => package,
        None => default_signed_package_path(&args.project)?,
    };
    let root_public_key = args
        .root_public_key
        .ok_or_else(|| anyhow::anyhow!("--root-public-key is required for MPKG verification"))?;
    let unix_time = args
        .unix_time
        .ok_or_else(|| anyhow::anyhow!("--unix-time is required for MPKG verification"))?;

    let status = Command::new("msign")
        .arg("package")
        .arg("verify")
        .arg(&package)
        .arg("--root-public-key")
        .arg(root_public_key)
        .arg("--unix-time")
        .arg(unix_time.to_string())
        .status()
        .context("failed to execute msign. is msign installed?")?;

    if !status.success() {
        bail!("msign package verify failed");
    }

    Ok(())
}
