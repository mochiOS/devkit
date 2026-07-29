use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KomeManifest {
    pub package: Package,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer: Option<Developer>,
    pub app: App,
    pub resources: Resources,
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub id: String,
    pub version: String,
    pub vendor: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Developer {
    pub id: String,
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
    pub fn new_app(name: String, id: String, vendor: String) -> Self {
        Self {
            package: Package {
                name,
                id,
                version: "0.1.0".to_string(),
                vendor,
                description: String::new(),
            },
            developer: None,
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
