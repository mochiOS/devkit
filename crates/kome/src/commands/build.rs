use std::{
    fs::{self, File},
    io::Write,
};

use anyhow::{bail, Context, Result};

use crate::{cli::BuildArgs, project};

pub fn run(args: BuildArgs) -> Result<()> {
    let project_dir = args.project_dir;
    let manifest = project::read_manifest(&project_dir)?;

    let source = project_dir.join("src/main.kome");
    if !source.exists() {
        bail!("source file does not exist: {}", source.display());
    }

    let out_dir = project_dir.join("target/debug");
    fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create {}", out_dir.display()))?;

    let entry_path = out_dir.join(&manifest.app.entry);

    write_mock_elf(&entry_path, &manifest.package.name)?;

    println!("built: {}", entry_path.display());
    println!("note: komec is not implemented yet, generated mock ELF");

    Ok(())
}

fn write_mock_elf(path: &std::path::Path, app_name: &str) -> Result<()> {
    let mut file = File::create(path)
        .with_context(|| format!("failed to create {}", path.display()))?;

    file.write_all(b"\x7fELF")
        .context("failed to write ELF magic")?;

    file.write_all(b"\n")
        .context("failed to write mock ELF")?;

    file.write_all(format!("mock executable for {}\n", app_name).as_bytes())
        .context("failed to write mock ELF body")?;

    Ok(())
}