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
        .unwrap_or_else(|| format!("com.example.{}", package_segment(&args.name)));

    let manifest = KomeManifest::new_app(args.name.clone(), id, args.vendor);

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

fn package_segment(name: &str) -> String {
    let mut segment = String::new();
    let mut pending_hyphen = false;
    for byte in name.bytes() {
        let byte = byte.to_ascii_lowercase();
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            if pending_hyphen && !segment.is_empty() {
                segment.push('-');
            }
            segment.push(char::from(byte));
            pending_hyphen = false;
        } else if !segment.is_empty() {
            pending_hyphen = true;
        }
    }
    if segment.is_empty() {
        "application".to_string()
    } else {
        segment
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_package_segment_is_canonical() {
        assert_eq!(package_segment("My Paint App"), "my-paint-app");
        assert_eq!(package_segment("--Volume__Control--"), "volume-control");
        assert_eq!(package_segment("日本語"), "application");
    }
}
