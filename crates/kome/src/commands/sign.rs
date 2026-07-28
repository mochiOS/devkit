use std::{path::PathBuf, process::Command};

use anyhow::{bail, Context, Result};

use crate::{cli::SignArgs, project};

pub fn run(args: SignArgs) -> Result<()> {
    if args.legacy {
        return run_legacy(args);
    }

    let package = match args.package {
        Some(package) => package,
        None => default_unsigned_package_path(&args.project)?,
    };
    let output = args
        .output
        .unwrap_or(default_signed_package_path(&args.project)?);

    let mut command = Command::new("msign");
    command
        .arg("package")
        .arg("sign")
        .arg(&package)
        .arg("--certificate")
        .arg(&args.certificate)
        .arg("--key")
        .arg(&args.key)
        .arg("--output")
        .arg(&output);
    if let Some(unix_time) = args.unix_time {
        command.arg("--unix-time").arg(unix_time.to_string());
    }

    let status = command
        .status()
        .context("failed to execute msign. is msign installed?")?;

    if !status.success() {
        bail!("msign package sign failed");
    }

    Ok(())
}

fn run_legacy(args: SignArgs) -> Result<()> {
    eprintln!("warning: legacy .pkg signing does not support mochiOS AppStore");
    let package = match args.package {
        Some(package) => package,
        None => default_legacy_package_path(&args.project)?,
    };

    let status = Command::new("msign")
        .arg("sign")
        .arg(&package)
        .arg("--key")
        .arg(&args.key)
        .arg("--key-id")
        .arg(&args.key_id)
        .status()
        .context("failed to execute msign. is msign installed?")?;

    if !status.success() {
        bail!("msign failed");
    }

    Ok(())
}

fn default_legacy_package_path(project_dir: &std::path::Path) -> Result<PathBuf> {
    let manifest = project::read_manifest(project_dir)?;

    Ok(project_dir
        .join("target/package")
        .join(format!("{}.pkg", manifest.package.name)))
}

fn default_unsigned_package_path(project_dir: &std::path::Path) -> Result<PathBuf> {
    let manifest = project::read_manifest(project_dir)?;
    Ok(project_dir
        .join("dist")
        .join(format!("{}-unsigned.mpkg", manifest.package.name)))
}

pub fn default_signed_package_path(project_dir: &std::path::Path) -> Result<PathBuf> {
    let manifest = project::read_manifest(project_dir)?;
    Ok(project_dir
        .join("dist")
        .join(format!("{}.mpkg", manifest.package.name)))
}
