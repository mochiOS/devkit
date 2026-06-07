use std::{path::PathBuf, process::Command};

use anyhow::{bail, Context, Result};

use crate::{cli::SignArgs, project};

pub fn run(args: SignArgs) -> Result<()> {
    let package = match args.package {
        Some(package) => package,
        None => default_package_path(&args.project)?,
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

fn default_package_path(project_dir: &std::path::Path) -> Result<PathBuf> {
    let manifest = project::read_manifest(project_dir)?;

    Ok(project_dir
        .join("target/package")
        .join(format!("{}.pkg", manifest.package.name)))
}