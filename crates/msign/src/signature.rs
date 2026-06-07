use serde::{Deserialize, Serialize};

pub const SIGNATURE_PATH: &str = "META/signature.toml";
pub const SIGNATURE_VERSION: u32 = 1;
pub const SIGNATURE_ALGORITHM: &str = "ed25519";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSignature {
    pub version: u32,
    pub algorithm: String,
    pub key_id: String,
    pub public_key: String,
    pub package_hash: String,
    pub signature: String,
}

impl PackageSignature {
    pub fn signing_message(package_hash: &str, key_id: &str, public_key: &str) -> Vec<u8> {
        format!(
            "mochios-package-signature-v1\npackage_hash={}\nkey_id={}\npublic_key={}\n",
            package_hash, key_id, public_key
        )
            .into_bytes()
    }
}