use std::{fs, path::Path};

use anyhow::{Context, Result};

use crate::manifest::KomeManifest;

pub const MANIFEST_FILE: &str = "Kome.toml";

pub fn read_manifest(project_dir: &Path) -> Result<KomeManifest> {
    let path = project_dir.join(MANIFEST_FILE);

    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    toml::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))
}

pub fn write_manifest(project_dir: &Path, manifest: &KomeManifest) -> Result<()> {
    let path = project_dir.join(MANIFEST_FILE);

    let text = toml::to_string_pretty(manifest)
        .context("failed to serialize Kome.toml")?;

    fs::write(&path, text)
        .with_context(|| format!("failed to write {}", path.display()))
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("failed to create {}", path.display()))
}