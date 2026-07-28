use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::cli::KeygenArgs;

pub fn run(args: KeygenArgs) -> Result<()> {
    let status = Command::new("msign")
        .arg("keygen")
        .arg("--private-key")
        .arg(&args.private_key)
        .arg("--public-key")
        .arg(&args.public_key)
        .status()
        .context("failed to execute msign. is msign installed?")?;

    if !status.success() {
        bail!("msign keygen failed");
    }

    Ok(())
}
