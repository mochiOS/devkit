use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct KomeManifest {
    pub package: Package,
    pub app: App,
    pub resources: Resources,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub name: String,
    pub id: String,
    pub version: String,
    pub developer: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct App {
    pub entry: String,
    pub icon: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Resources {
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Capabilities {
    pub required: Vec<String>,
    pub optional: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct AboutToml {
    pub name: String,
    pub bundle_id: String,
    pub version: String,
    pub developer: String,
    pub entry: String,
    pub description: String,
    pub icon: String,
    pub resources: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RuntimeManifestToml {
    pub app: RuntimeApp,
    pub capabilities: RuntimeCapabilities,
}

#[derive(Debug, Serialize)]
pub struct RuntimeApp {
    pub id: String,
    pub name: String,
    pub entry: String,
}

#[derive(Debug, Serialize)]
pub struct RuntimeCapabilities {
    pub required: Vec<String>,
    pub optional: Vec<String>,
}

pub fn read_kome_manifest(project_dir: &Path) -> Result<KomeManifest> {
    let path = project_dir.join("Kome.toml");

    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;

    toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn make_about_toml(manifest: &KomeManifest) -> AboutToml {
    AboutToml {
        name: manifest.package.name.clone(),
        bundle_id: manifest.package.id.clone(),
        version: manifest.package.version.clone(),
        developer: manifest.package.developer.clone(),
        entry: manifest.app.entry.clone(),
        description: manifest.package.description.clone(),
        icon: manifest.app.icon.clone(),
        resources: manifest.resources.files.clone(),
    }
}

pub fn make_runtime_manifest(manifest: &KomeManifest) -> RuntimeManifestToml {
    RuntimeManifestToml {
        app: RuntimeApp {
            id: manifest.package.id.clone(),
            name: manifest.package.name.clone(),
            entry: format!(
                "/applications/{}.app/{}",
                manifest.package.name, manifest.app.entry
            ),
        },
        capabilities: RuntimeCapabilities {
            required: manifest.capabilities.required.clone(),
            optional: manifest.capabilities.optional.clone(),
        },
    }
}
