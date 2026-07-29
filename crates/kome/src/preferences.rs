use std::{fs, io::Write, path::PathBuf};

use anyhow::{bail, Context, Result};
use mochios_certificate::is_valid_developer_id;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::credential::config_dir;

const SETTINGS_FILE: &str = "settings.toml";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Preferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_developer: Option<String>,
}

impl Preferences {
    pub fn load() -> Result<Self> {
        Self::load_from(settings_path()?)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(settings_path()?)
    }

    fn load_from(path: PathBuf) -> Result<Self> {
        match fs::read_to_string(&path) {
            Ok(text) => {
                let value: Self = toml::from_str(&text)
                    .with_context(|| format!("failed to parse {}", path.display()))?;
                value.validate()?;
                Ok(value)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
        }
    }

    fn save_to(&self, path: PathBuf) -> Result<()> {
        self.validate()?;
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("settings path has no parent"))?;
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
        let mut temporary = NamedTempFile::new_in(parent)
            .with_context(|| format!("failed to create a file in {}", parent.display()))?;
        protect(temporary.path())?;
        temporary
            .write_all(toml::to_string_pretty(self)?.as_bytes())
            .context("failed to write Kome settings")?;
        temporary
            .persist(&path)
            .map_err(|error| error.error)
            .with_context(|| format!("failed to replace {}", path.display()))?;
        protect(&path)
    }

    fn validate(&self) -> Result<()> {
        if let Some(developer_id) = &self.default_developer {
            if !is_valid_developer_id(developer_id) {
                bail!("default Developer ID is invalid");
            }
        }
        Ok(())
    }
}

fn settings_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(SETTINGS_FILE))
}

#[cfg(unix)]
fn protect(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn protect(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_reject_old_developer_prefix() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("settings.toml");
        fs::write(
            &path,
            "default_developer = \"org.mochios.developer.019f9e5ac6687902b0e72fe53abfbef1\"\n",
        )
        .unwrap();
        assert!(Preferences::load_from(path).is_err());
    }

    #[test]
    fn settings_round_trip_canonical_developer() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("settings.toml");
        let expected = Preferences {
            default_developer: Some("019f9e5ac6687902b0e72fe53abfbef1".to_string()),
        };
        expected.save_to(path.clone()).unwrap();
        assert_eq!(Preferences::load_from(path).unwrap(), expected);
    }
}
