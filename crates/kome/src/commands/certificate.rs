use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::{cli::CertificateObtainArgs, project};

pub fn obtain(args: CertificateObtainArgs) -> Result<()> {
    let package = match args.package {
        Some(package) => package,
        None => default_unsigned_package_path(&args.project)?,
    };
    let mut command = Command::new("msign");
    command
        .arg("certificate")
        .arg("obtain")
        .arg("--developer")
        .arg(&args.developer)
        .arg("--public-key")
        .arg(&args.public_key)
        .arg("--package")
        .arg(&package)
        .arg("--output")
        .arg(&args.output)
        .arg("--api-base")
        .arg(&args.api_base);
    if let Some(token) = args.bearer_token {
        command.arg("--bearer-token").arg(token);
    }
    if let Some(key) = args.idempotency_key {
        command.arg("--idempotency-key").arg(key);
    }
    let status = command
        .status()
        .context("failed to execute msign. is msign installed?")?;
    if !status.success() {
        bail!("msign certificate obtain failed");
    }
    Ok(())
}

fn default_unsigned_package_path(project_dir: &std::path::Path) -> Result<std::path::PathBuf> {
    let manifest = project::read_manifest(project_dir)?;
    Ok(project_dir
        .join("dist")
        .join(format!("{}-unsigned.mpkg", manifest.package.name)))
}
