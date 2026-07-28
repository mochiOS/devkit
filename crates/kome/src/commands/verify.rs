use std::{env, path::PathBuf, process::Command};

use anyhow::{bail, Context, Result};

use crate::{cli::VerifyArgs, commands::sign::default_signed_package_path, project};

pub fn run(args: VerifyArgs) -> Result<()> {
    if args.legacy {
        return run_legacy(args);
    }

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

fn run_legacy(args: VerifyArgs) -> Result<()> {
    eprintln!("warning: legacy .pkg verification does not support mochiOS AppStore");
    let package = match args.package {
        Some(package) => package,
        None => default_legacy_package_path()?,
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

fn default_legacy_package_path() -> Result<PathBuf> {
    let project_dir = env::current_dir().context("failed to get current directory")?;
    let manifest = project::read_manifest(&project_dir)?;

    Ok(project_dir
        .join("target/package")
        .join(format!("{}.pkg", manifest.package.name)))
}
