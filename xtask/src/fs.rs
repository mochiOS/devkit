use std::{fs, path::Path};

use anyhow::{Context, Result};

pub fn recreate_dir(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path).with_context(|| format!("failed to remove {}", path.display()))?;
    }

    fs::create_dir_all(path).with_context(|| format!("failed to create {}", path.display()))?;

    Ok(())
}

pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    Ok(())
}
