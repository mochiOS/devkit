use std::{fs, path::Path};

use anyhow::{bail, Context, Result};
use mochios_certificate::{is_valid_developer_id, is_valid_package_id};

use crate::manifest::KomeManifest;

pub const MANIFEST_FILE: &str = "Kome.toml";

pub fn read_manifest(project_dir: &Path) -> Result<KomeManifest> {
    let path = project_dir.join(MANIFEST_FILE);

    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;

    let manifest: KomeManifest =
        toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn validate_manifest(manifest: &KomeManifest) -> Result<()> {
    if !is_valid_package_id(&manifest.package.id) {
        bail!("package.id is not a canonical Package ID");
    }
    if let Some(developer) = &manifest.developer {
        if !is_valid_developer_id(&developer.id) {
            bail!("developer.id must be a 32-character lowercase hexadecimal identifier");
        }
    }
    Ok(())
}

pub fn write_manifest(project_dir: &Path, manifest: &KomeManifest) -> Result<()> {
    validate_manifest(manifest)?;
    let path = project_dir.join(MANIFEST_FILE);

    let text = toml::to_string_pretty(manifest).context("failed to serialize Kome.toml")?;

    fs::write(&path, text).with_context(|| format!("failed to write {}", path.display()))
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(package_id: &str) -> KomeManifest {
        KomeManifest::new_app(
            "Example".to_string(),
            package_id.to_string(),
            "Example Developer".to_string(),
        )
    }

    #[test]
    fn package_id_rules_are_enforced_by_project_validation() {
        for valid in ["com.example.app", "io.github.user.app", "org.mochios.app"] {
            assert!(validate_manifest(&manifest(valid)).is_ok(), "{valid}");
        }
        for invalid in [
            "application",
            "Com.example.app",
            "com..example",
            "com.example_app",
            "com.-example",
            "com.example-",
        ] {
            assert!(validate_manifest(&manifest(invalid)).is_err(), "{invalid}");
        }
    }
}
