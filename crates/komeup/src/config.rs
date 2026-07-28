use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct KomeupConfig {
    pub version: u32,
    pub home: String,
    pub default_toolchain: String,

    #[serde(default)]
    pub toolchains: BTreeMap<String, ToolchainInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolchainInfo {
    pub channel: String,
    pub version: String,
    pub path: String,
    pub installed_at: String,

    #[serde(default)]
    pub components: BTreeMap<String, ComponentInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ComponentInfo {
    pub name: String,
    pub version: String,
    pub path: String,
    pub source: String,
}

impl KomeupConfig {
    pub fn new(home: &Path) -> Self {
        Self {
            version: CONFIG_VERSION,
            home: home.to_string_lossy().to_string(),
            default_toolchain: "stable".to_string(),
            toolchains: BTreeMap::new(),
        }
    }
}

pub fn kome_home() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os("KOMEUP_HOME") {
        return Ok(PathBuf::from(value));
    }

    let home = dirs::home_dir().context("failed to detect home directory")?;
    Ok(home.join(".kome"))
}

pub fn config_path(home: &Path) -> PathBuf {
    home.join("komeup.toml")
}

pub fn load_config(home: &Path) -> Result<KomeupConfig> {
    let path = config_path(home);

    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;

    let config = toml::from_str::<KomeupConfig>(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;

    Ok(config)
}

pub fn load_or_create_config(home: &Path) -> Result<KomeupConfig> {
    let path = config_path(home);

    if !path.exists() {
        return Ok(KomeupConfig::new(home));
    }

    load_config(home)
}

pub fn save_config(home: &Path, config: &KomeupConfig) -> Result<()> {
    fs::create_dir_all(home).with_context(|| format!("failed to create {}", home.display()))?;

    let path = config_path(home);

    let text = toml::to_string_pretty(config).context("failed to serialize komeup config")?;

    fs::write(&path, text).with_context(|| format!("failed to write {}", path.display()))?;

    Ok(())
}

pub fn path_string(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().to_string()
}
