use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::cli::PackArgs;

pub fn run(args: PackArgs) -> Result<()> {
    let mut command = Command::new("mpack");

    command.arg("pack");
    command.arg(&args.project_dir);

    if let Some(output) = args.output {
        command.arg("--output");
        command.arg(output);
    }

    if args.release {
        command.arg("--release");
    }

    if args.force {
        command.arg("--force");
    }

    let status = command
        .status()
        .context("failed to execute mpack. is mpack installed?")?;

    if !status.success() {
        bail!("mpack failed");
    }

    Ok(())
}