use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KomeManifest {
    pub package: Package,
    pub app: App,
    pub resources: Resources,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub id: String,
    pub version: String,
    pub developer: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub entry: String,
    pub icon: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resources {
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub required: Vec<String>,
    pub optional: Vec<String>,
}

impl KomeManifest {
    pub fn new_app(name: String, id: String, developer: String) -> Self {
        Self {
            package: Package {
                name,
                id,
                version: "0.1.0".to_string(),
                developer,
                description: String::new(),
            },
            app: App {
                entry: "entry.elf".to_string(),
                icon: "assets/icon.png".to_string(),
            },
            resources: Resources { files: vec![] },
            capabilities: Capabilities {
                required: vec![],
                optional: vec![],
            },
        }
    }
}
