use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};
use mochios_signature_protocol::{InstallProvenance, InstallRecord, VerifiedResponse};
use sha2::{Digest, Sha256};

use crate::cli::BuiltInRecordArgs;

const BUILT_IN_DEVELOPER_ID: &str = "org.mochios.system";

pub(super) fn read_manifest(
    path: &Path,
) -> Result<(Vec<u8>, mochios_package_manifest::PackageManifest)> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    if !file.metadata()?.is_file() {
        bail!("manifest is not a regular file");
    }
    let mut manifest_bytes = Vec::new();
    file.take(mochios_package_manifest::MAX_MANIFEST_LEN as u64 + 1)
        .read_to_end(&mut manifest_bytes)?;
    let manifest_text = std::str::from_utf8(&manifest_bytes)
        .with_context(|| format!("{} is not UTF-8", path.display()))?;
    let manifest = mochios_package_manifest::parse_manifest_checked(manifest_text)
        .map_err(|e| anyhow::anyhow!("invalid manifest {}: {e:?}", path.display()))?;
    if manifest.install_provenance.as_deref() != Some("built-in") {
        bail!("built-in record requires package.provenance = \"built-in\"");
    }
    Ok((manifest_bytes, manifest))
}

pub(crate) fn record(args: BuiltInRecordArgs) -> Result<()> {
    let (manifest_bytes, manifest) = read_manifest(&args.manifest)?;
    let mut capabilities = BTreeSet::new();
    for binary in manifest.binaries {
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
        verified_package_id: &manifest.package_id,
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
