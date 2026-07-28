use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};

use crate::manifest::KomeManifest;

#[derive(Debug, Clone)]
pub struct PackageFile {
    pub source: PathBuf,
    pub dest: PathBuf,
}

pub fn collect_package_files(
    project_dir: &Path,
    build_dir: &Path,
    manifest: &KomeManifest,
) -> Result<Vec<PackageFile>> {
    validate_manifest_paths(project_dir, manifest)?;

    let mut files = Vec::new();

    let entry_source = build_dir.join(&manifest.app.entry);
    if !entry_source.exists() {
        bail!(
            "entry file does not exist: {}. run `kome build` first",
            entry_source.display()
        );
    }

    files.push(PackageFile {
        source: entry_source,
        dest: PathBuf::from(&manifest.app.entry),
    });

    let icon_source = project_dir.join(&manifest.app.icon);
    if icon_source.exists() {
        files.push(PackageFile {
            source: icon_source,
            dest: PathBuf::from(&manifest.app.icon),
        });
    }

    for resource in &manifest.resources.files {
        let source = project_dir.join(resource);

        if !source.exists() {
            bail!("resource does not exist: {}", source.display());
        }

        files.push(PackageFile {
            source,
            dest: PathBuf::from(resource),
        });
    }

    dedup_files(files)
}

fn validate_manifest_paths(project_dir: &Path, manifest: &KomeManifest) -> Result<()> {
    validate_relative_path(&manifest.app.entry, "app.entry")?;
    validate_relative_path(&manifest.app.icon, "app.icon")?;

    for resource in &manifest.resources.files {
        validate_relative_path(resource, "resources.files")?;

        let path = project_dir.join(resource);
        if path.is_dir() {
            bail!(
                "resource directories are not supported yet: {}",
                path.display()
            );
        }
    }

    Ok(())
}

fn validate_relative_path(path: &str, field: &str) -> Result<()> {
    if path.trim().is_empty() {
        bail!("{} is empty", field);
    }

    if path.starts_with('/') || path.contains("..") {
        bail!("{} must be a relative path inside project", field);
    }

    Ok(())
}

fn dedup_files(files: Vec<PackageFile>) -> Result<Vec<PackageFile>> {
    let mut result: Vec<PackageFile> = Vec::new();

    for file in files {
        if result.iter().any(|existing| existing.dest == file.dest) {
            continue;
        }

        let metadata = fs::metadata(&file.source)
            .with_context(|| format!("failed to stat {}", file.source.display()))?;

        if !metadata.is_file() {
            bail!("package input is not a file: {}", file.source.display());
        }

        result.push(file);
    }

    Ok(result)
}
