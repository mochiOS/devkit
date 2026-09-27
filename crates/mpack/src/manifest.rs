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
pub struct RuntimeManifestToml {
    pub format: u32,
    pub package: RuntimePackage,
    pub application: RuntimeApplication,
    pub capabilities: RuntimeCapabilities,
}

#[derive(Debug, Serialize)]
pub struct RuntimePackage {
    pub id: String,
    pub name: String,
    pub version: String,
    pub vendor: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct RuntimeApplication {
    pub entry: String,
    pub description: String,
    pub icon: String,
    pub resources: Vec<String>,
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

pub fn make_runtime_manifest(manifest: &KomeManifest) -> RuntimeManifestToml {
    RuntimeManifestToml {
        format: 1,
        package: RuntimePackage {
            id: manifest.package.id.clone(),
            name: manifest.package.name.clone(),
            version: manifest.package.version.clone(),
            vendor: manifest.package.developer.clone(),
            kind: String::from("application"),
        },
        application: RuntimeApplication {
            entry: manifest.app.entry.clone(),
            description: manifest.package.description.clone(),
            icon: manifest.app.icon.clone(),
            resources: manifest.resources.files.clone(),
        },
        capabilities: RuntimeCapabilities {
            required: manifest.capabilities.required.clone(),
            optional: manifest.capabilities.optional.clone(),
        },
    }
}
