use std::collections::BTreeSet;
use std::fs;

use anyhow::{Context, Result, bail};
use mochios_signature_protocol::{InstallProvenance, InstallRecord, VerifiedResponse};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::cli::BuiltInRecordArgs;

const BUILT_IN_DEVELOPER_ID: &str = "org.mochios.system";

#[derive(Deserialize)]
struct ManifestDocument {
    package: ManifestPackage,
    #[serde(default)]
    binary: Vec<ManifestBinary>,
}

#[derive(Deserialize)]
struct ManifestPackage {
    id: String,
    provenance: String,
}

#[derive(Deserialize)]
struct ManifestBinary {
    #[serde(default)]
    requires: Vec<String>,
}

pub(crate) fn record(args: BuiltInRecordArgs) -> Result<()> {
    let manifest_bytes = fs::read(&args.manifest)
        .with_context(|| format!("failed to read {}", args.manifest.display()))?;
    let manifest_text = std::str::from_utf8(&manifest_bytes)
        .with_context(|| format!("{} is not UTF-8", args.manifest.display()))?;
    let manifest: ManifestDocument = toml::from_str(manifest_text)
        .with_context(|| format!("failed to parse {}", args.manifest.display()))?;
    if manifest.package.provenance != "built-in" {
        bail!("built-in record requires package.provenance = \"built-in\"");
    }
    if manifest.package.id.is_empty()
        || manifest
            .package
            .id
            .bytes()
            .any(|byte| byte == 0 || byte.is_ascii_control())
    {
        bail!("invalid built-in Package ID");
    }

    let mut capabilities = BTreeSet::new();
    for binary in manifest.binary {
        for capability in binary.requires {
            if capability.is_empty()
                || capability
                    .bytes()
                    .any(|byte| byte == 0 || byte.is_ascii_control())
            {
                bail!("invalid capability in built-in manifest");
            }
            capabilities.insert(capability);
        }
    }
    let capabilities = capabilities.into_iter().collect::<Vec<_>>();
    let capability_refs = capabilities.iter().map(String::as_str).collect::<Vec<_>>();
    let digest: [u8; 32] = Sha256::digest(&manifest_bytes).into();
    let verification = VerifiedResponse {
        request_id: 0,
        provenance: InstallProvenance::BuiltIn,
        certificate_serial: 0,
        subject_key_id: [0; 32],
        manifest_digest: digest,
        package_digest: digest,
        developer_id: BUILT_IN_DEVELOPER_ID,
        verified_package_id: &manifest.package.id,
        allowed_capabilities: &capability_refs,
    };
    let record = InstallRecord {
        provenance: InstallProvenance::BuiltIn,
        verification,
    };
    let mut encoded = vec![0; record.encoded_len()];
    let length = record
        .encode(&mut encoded)
        .map_err(|error| anyhow::anyhow!("failed to encode built-in install record: {error:?}"))?;
    encoded.truncate(length);
    fs::write(&args.output, encoded)
        .with_context(|| format!("failed to write {}", args.output.display()))
}
