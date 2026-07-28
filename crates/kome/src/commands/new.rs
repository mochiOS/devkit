use std::fs;

use anyhow::{bail, Context, Result};

use crate::{cli::NewArgs, manifest::KomeManifest, project};

pub fn run(args: NewArgs) -> Result<()> {
    let project_dir = std::env::current_dir()?.join(&args.name);

    if project_dir.exists() {
        bail!("project already exists: {}", project_dir.display());
    }

    project::ensure_dir(&project_dir)?;
    project::ensure_dir(&project_dir.join("src"))?;
    project::ensure_dir(&project_dir.join("assets"))?;

    let id = args
        .id
        .unwrap_or_else(|| format!("com.example.{}", args.name.to_lowercase()));

    let manifest = KomeManifest::new_app(args.name.clone(), id, args.developer);

    project::write_manifest(&project_dir, &manifest)?;

    fs::write(
        project_dir.join("src/main.kome"),
        include_str!("../templates/main.kome"),
    )
    .context("failed to write src/main.kome")?;

    fs::write(
        project_dir.join(".gitignore"),
        "target/\ndist/\nkeys/*.key\n",
    )
    .context("failed to write .gitignore")?;

    println!("created project: {}", project_dir.display());

    Ok(())
}
