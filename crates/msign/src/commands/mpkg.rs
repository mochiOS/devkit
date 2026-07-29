use std::collections::BTreeSet;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use ed25519_dalek::{Signature, Signer, VerifyingKey};
use mochios_certificate::{is_valid_package_id, DeveloperCertificate, SIGNATURE_LEN};
use sha2::{Digest, Sha256};
use tar::{Archive, Builder, EntryType, Header};
use tempfile::NamedTempFile;

use crate::cli::{PackageSignArgs, PackageVerifyArgs};
use crate::commands::certificate::CertificateRequest;
use crate::crypto;

const MPKG_MAGIC: &[u8; 4] = b"MPKG";
const MPKG_HEADER_LEN: usize = 32;
const MAX_PACKAGE_LEN: u64 = 128 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
const MAX_METADATA_LEN: usize = 1024 * 1024;
const MANIFEST_PATH: &str = "manifest.toml";
const CERTIFICATE_PATH: &str = "signatures/developer.cert";
const MANIFEST_SIGNATURE_PATH: &str = "signatures/manifest.sig";
const MANIFEST_DOMAIN: &[u8] = b"mochios-mpkg-manifest-v1\0";

#[derive(Clone)]
struct MpkgEntry {
    path: String,
    data: Vec<u8>,
    mode: u32,
}

pub fn sign(args: PackageSignArgs) -> Result<()> {
    let mut entries = read_mpkg(&args.package)?;
    reject_chain_and_unknown_signatures(&entries)?;
    if !args.replace_signature
        && (entries.iter().any(|entry| entry.path == CERTIFICATE_PATH)
            || entries
                .iter()
                .any(|entry| entry.path == MANIFEST_SIGNATURE_PATH))
    {
        bail!("MPKG is already signed; use --replace-signature to replace signatures");
    }
    let manifest = entry(&entries, MANIFEST_PATH)?.data.clone();
    let manifest_text = std::str::from_utf8(&manifest).context("manifest is not UTF-8")?;
    let manifest_value: toml::Value =
        toml::from_str(manifest_text).context("manifest is not valid TOML")?;
    validate_manifest_shape(&manifest_value)?;
    let package_id = package_id(&manifest_value)?;
    let certificate_bytes = fs::read(&args.certificate)
        .with_context(|| format!("failed to read {}", args.certificate.display()))?;
    let certificate = decode_canonical_certificate(&certificate_bytes)?;
    let developer_key = crypto::read_private_key(&args.key)?;
    if developer_key.verifying_key().to_bytes() != certificate.subject_public_key {
        bail!("developer private key does not match certificate subject public key");
    }
    validate_certificate_for_manifest(&certificate, &manifest_value, package_id, args.unix_time)?;
    let signature = developer_key
        .sign(&manifest_signing_message(&manifest))
        .to_bytes();

    entries.retain(|entry| entry.path != CERTIFICATE_PATH && entry.path != MANIFEST_SIGNATURE_PATH);
    entries.push(MpkgEntry {
        path: CERTIFICATE_PATH.to_string(),
        data: certificate_bytes,
        mode: 0o644,
    });
    entries.push(MpkgEntry {
        path: MANIFEST_SIGNATURE_PATH.to_string(),
        data: signature.to_vec(),
        mode: 0o644,
    });
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let output = args.output.unwrap_or_else(|| args.package.clone());
    write_mpkg(&output, &entries)?;
    println!("signed: {}", output.display());
    println!("developer_id: {}", certificate.developer_id);
    println!("certificate_serial: {}", certificate.serial_number);
    Ok(())
}

pub(crate) fn certificate_request(
    package: &Path,
    developer_id: &str,
    public_key: &VerifyingKey,
) -> Result<CertificateRequest> {
    let package_bytes = read_package_bytes(package)?;
    let entries = parse_mpkg(&package_bytes)?;
    reject_chain_and_unknown_signatures(&entries)?;
    let manifest = &entry(&entries, MANIFEST_PATH)?.data;
    let manifest_text = std::str::from_utf8(manifest).context("manifest is not UTF-8")?;
    let manifest_value: toml::Value =
        toml::from_str(manifest_text).context("manifest is not valid TOML")?;
    validate_manifest_shape(&manifest_value)?;
    let package_id = package_id(&manifest_value)?;
    let mut capabilities = required_capabilities(&manifest_value)?;
    capabilities.sort();
    capabilities.dedup();
    Ok(CertificateRequest {
        developer_id: developer_id.to_string(),
        subject_public_key: crypto::public_key_to_base64(public_key),
        package_id: package_id.to_string(),
        capabilities,
    })
}

pub fn verify(args: PackageVerifyArgs) -> Result<()> {
    let package_bytes = read_package_bytes(&args.package)?;
    let entries = parse_mpkg(&package_bytes)?;
    reject_chain_and_unknown_signatures(&entries)?;
    let manifest = &entry(&entries, MANIFEST_PATH)?.data;
    let manifest_text = std::str::from_utf8(manifest).context("manifest is not UTF-8")?;
    let manifest_value: toml::Value =
        toml::from_str(manifest_text).context("manifest is not valid TOML")?;
    validate_manifest_shape(&manifest_value)?;
    let package_id = package_id(&manifest_value)?;
    let certificate_bytes = &entry(&entries, CERTIFICATE_PATH)?.data;
    let certificate = decode_canonical_certificate(certificate_bytes)?;
    let root_public_key = crypto::read_public_key(&args.root_public_key)?.to_bytes();
    certificate
        .verify(&root_public_key, args.unix_time, package_id)
        .map_err(|error| anyhow!(error))?;
    validate_certificate_capabilities_for_manifest(&certificate, &manifest_value)?;
    verify_manifest_signature(&certificate, manifest, &entries)?;
    verify_payload(&manifest_value, &entries)?;

    println!("verified: {}", args.package.display());
    println!("verified_package_id: {package_id}");
    println!("developer_id: {}", certificate.developer_id);
    println!("certificate_serial: {}", certificate.serial_number);
    println!("subject_key_id: {}", hex(&certificate.subject_key_id));
    println!("manifest_digest: {}", hex(&Sha256::digest(manifest)));
    println!("package_digest: {}", hex(&Sha256::digest(&package_bytes)));
    for capability in certificate.allowed_capabilities {
        println!("allowed_capability: {capability}");
    }
    Ok(())
}

fn read_mpkg(path: &Path) -> Result<Vec<MpkgEntry>> {
    let bytes = read_package_bytes(path)?;
    parse_mpkg(&bytes)
}

fn read_package_bytes(path: &Path) -> Result<Vec<u8>> {
    let length = fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .len();
    if length > MAX_PACKAGE_LEN {
        bail!("MPKG exceeds AppStore Reviewer package size limit");
    }
    fs::read(path).with_context(|| format!("failed to read {}", path.display()))
}

fn parse_mpkg(bytes: &[u8]) -> Result<Vec<MpkgEntry>> {
    if bytes.len() < MPKG_HEADER_LEN || &bytes[..4] != MPKG_MAGIC {
        bail!("invalid MPKG header");
    }
    if read_u16(bytes, 4) != 1 {
        bail!("unsupported MPKG major version");
    }
    if read_u16(bytes, 6) != 0 {
        bail!("unsupported MPKG minor version");
    }
    if usize::from(read_u16(bytes, 8)) != MPKG_HEADER_LEN {
        bail!("invalid MPKG header length");
    }
    if bytes[10] != 0 {
        bail!("compressed MPKG is not supported");
    }
    if bytes[11] != 0 || bytes[20..32].iter().any(|byte| *byte != 0) {
        bail!("unknown MPKG flags or non-zero reserved field");
    }
    let expanded_size = read_u64(bytes, 12) as usize;
    let tar_bytes = &bytes[MPKG_HEADER_LEN..];
    if tar_bytes.len() != expanded_size {
        bail!("MPKG expanded size does not match payload length");
    }
    validate_ustar_stream(tar_bytes)?;

    let mut archive = Archive::new(Cursor::new(tar_bytes));
    let mut paths = BTreeSet::new();
    let mut result = Vec::new();
    for item in archive
        .entries()
        .context("failed to parse MPKG tar stream")?
    {
        let mut item = item.context("failed to read MPKG entry")?;
        let entry_type = item.header().entry_type();
        if entry_type == EntryType::Directory {
            continue;
        }
        if !entry_type.is_file() {
            bail!("MPKG contains unsupported tar entry type");
        }
        let path = normalize_path(&item.path().context("invalid MPKG entry path")?)?;
        if path != MANIFEST_PATH
            && !path.starts_with("signatures/")
            && !path.starts_with("payload/")
        {
            bail!("MPKG contains entry outside allowed roots: {path}");
        }
        if path.starts_with("signatures/")
            && path != CERTIFICATE_PATH
            && path != MANIFEST_SIGNATURE_PATH
        {
            bail!("unknown MPKG signature entry: {path}");
        }
        if !paths.insert(path.clone()) {
            bail!("MPKG contains duplicate entry: {path}");
        }
        let mode = item.header().mode().unwrap_or(0o644);
        let mut data = Vec::new();
        item.read_to_end(&mut data)
            .context("failed to read MPKG entry data")?;
        result.push(MpkgEntry { path, data, mode });
    }
    Ok(result)
}

fn validate_ustar_stream(bytes: &[u8]) -> Result<()> {
    let mut paths = BTreeSet::new();
    let mut offset = 0usize;
    let mut entry_count = 0usize;
    while offset + 512 <= bytes.len() {
        let block = &bytes[offset..offset + 512];
        if block.iter().all(|byte| *byte == 0) {
            if bytes[offset..].iter().any(|byte| *byte != 0) {
                bail!("MPKG tar stream contains data after terminator");
            }
            return Ok(());
        }
        if entry_count >= MAX_ENTRIES {
            bail!("MPKG contains more than {MAX_ENTRIES} entries");
        }
        entry_count += 1;
        if &block[257..263] != b"ustar\0" || &block[263..265] != b"00" {
            bail!("MPKG tar entry is not ustar");
        }
        let expected_checksum = parse_tar_octal(&block[148..156])? as u64;
        let actual_checksum = tar_header_checksum(block);
        if expected_checksum != actual_checksum {
            bail!("MPKG tar entry checksum mismatch");
        }
        let kind = block[156];
        if kind != b'0' && kind != 0 && kind != b'5' {
            bail!("MPKG contains unsupported tar entry type");
        }
        let name = tar_cstr(&block[0..100])?;
        let prefix = tar_cstr(&block[345..500])?;
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        normalize_path(Path::new(&path))?;
        if path != MANIFEST_PATH
            && !path.starts_with("signatures/")
            && !path.starts_with("payload/")
        {
            bail!("MPKG contains entry outside allowed roots: {path}");
        }
        if path.starts_with("signatures/")
            && path != CERTIFICATE_PATH
            && path != MANIFEST_SIGNATURE_PATH
        {
            bail!("unknown MPKG signature entry: {path}");
        }
        if !paths.insert(path.clone()) {
            bail!("MPKG contains duplicate entry: {path}");
        }
        let size = parse_tar_octal(&block[124..136])?;
        if matches!(
            path.as_str(),
            MANIFEST_PATH | CERTIFICATE_PATH | MANIFEST_SIGNATURE_PATH
        ) && size > MAX_METADATA_LEN
        {
            bail!("MPKG metadata entry is too large: {path}");
        }
        let payload_start = offset
            .checked_add(512)
            .ok_or_else(|| anyhow!("MPKG tar stream is too large"))?;
        let payload_end = payload_start
            .checked_add(size)
            .ok_or_else(|| anyhow!("MPKG tar stream is too large"))?;
        if payload_end > bytes.len() {
            bail!("MPKG tar entry payload exceeds stream length");
        }
        let padded_size = size
            .checked_add(511)
            .map(|value| value / 512 * 512)
            .ok_or_else(|| anyhow!("MPKG tar entry is too large"))?;
        offset = payload_start
            .checked_add(padded_size)
            .ok_or_else(|| anyhow!("MPKG tar stream is too large"))?;
    }
    if offset != bytes.len() && bytes[offset..].iter().any(|byte| *byte != 0) {
        bail!("MPKG tar stream has trailing partial block");
    }
    Ok(())
}

fn tar_cstr(bytes: &[u8]) -> Result<String> {
    let len = match bytes.iter().position(|byte| *byte == 0) {
        Some(index) => index,
        None => bytes.len(),
    };
    std::str::from_utf8(&bytes[..len])
        .context("MPKG tar path is not UTF-8")
        .map(str::to_string)
}

fn parse_tar_octal(bytes: &[u8]) -> Result<usize> {
    let mut value = 0usize;
    let mut seen = false;
    for byte in bytes {
        if *byte == 0 || *byte == b' ' {
            break;
        }
        if !(b'0'..=b'7').contains(byte) {
            bail!("MPKG tar entry has invalid size");
        }
        seen = true;
        value = value
            .checked_mul(8)
            .and_then(|current| current.checked_add(usize::from(*byte - b'0')))
            .ok_or_else(|| anyhow!("MPKG tar entry is too large"))?;
    }
    if !seen {
        return Ok(0);
    }
    Ok(value)
}

fn tar_header_checksum(block: &[u8]) -> u64 {
    block
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                u64::from(b' ')
            } else {
                u64::from(*byte)
            }
        })
        .sum()
}

fn write_mpkg(path: &Path, entries: &[MpkgEntry]) -> Result<()> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = Builder::new(&mut tar_bytes);
        for entry in entries {
            let mut header = Header::new_ustar();
            header.set_size(entry.data.len() as u64);
            header.set_mode(entry.mode);
            header.set_uid(0);
            header.set_gid(0);
            header
                .set_username("root")
                .context("failed to set MPKG tar username")?;
            header
                .set_groupname("root")
                .context("failed to set MPKG tar groupname")?;
            header.set_mtime(0);
            header.set_entry_type(EntryType::Regular);
            header.set_cksum();
            builder
                .append_data(&mut header, &entry.path, Cursor::new(&entry.data))
                .with_context(|| format!("failed to append {}", entry.path))?;
        }
        builder
            .finish()
            .context("failed to finish MPKG tar stream")?;
    }
    let expanded_size = u64::try_from(tar_bytes.len()).context("MPKG is too large")?;
    let mut header = [0u8; MPKG_HEADER_LEN];
    header[..4].copy_from_slice(MPKG_MAGIC);
    header[4..6].copy_from_slice(&1u16.to_le_bytes());
    header[8..10].copy_from_slice(&(MPKG_HEADER_LEN as u16).to_le_bytes());
    header[12..20].copy_from_slice(&expanded_size.to_le_bytes());

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut temporary = NamedTempFile::new_in(path.parent().unwrap_or_else(|| Path::new(".")))
        .context("failed to create temporary MPKG")?;
    temporary
        .write_all(&header)
        .and_then(|_| temporary.write_all(&tar_bytes))
        .context("failed to write temporary MPKG")?;
    temporary
        .flush()
        .context("failed to flush temporary MPKG")?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

fn reject_chain_and_unknown_signatures(entries: &[MpkgEntry]) -> Result<()> {
    for entry in entries {
        if entry.path.starts_with("signatures/chain/") {
            bail!("MPKG v1 does not support intermediate certificate chains");
        }
        if entry.path.starts_with("signatures/")
            && entry.path != CERTIFICATE_PATH
            && entry.path != MANIFEST_SIGNATURE_PATH
        {
            bail!("unknown MPKG signature entry: {}", entry.path);
        }
    }
    Ok(())
}

pub(crate) fn decode_canonical_certificate(bytes: &[u8]) -> Result<DeveloperCertificate> {
    let certificate = DeveloperCertificate::decode(bytes).map_err(|error| anyhow!(error))?;
    let mut encoded = vec![0; certificate.encoded_len().map_err(|error| anyhow!(error))?];
    certificate
        .encode(&mut encoded)
        .map_err(|error| anyhow!(error))?;
    if encoded != bytes {
        bail!("developer certificate is not canonical MCER encoding");
    }
    Ok(certificate)
}

fn verify_manifest_signature(
    certificate: &DeveloperCertificate,
    manifest: &[u8],
    entries: &[MpkgEntry],
) -> Result<()> {
    let signature_bytes: [u8; SIGNATURE_LEN] = entry(entries, MANIFEST_SIGNATURE_PATH)?
        .data
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("manifest.sig must contain exactly 64 bytes"))?;
    let verifier = VerifyingKey::from_bytes(&certificate.subject_public_key)
        .context("certificate contains invalid subject public key")?;
    verifier
        .verify_strict(
            &manifest_signing_message(manifest),
            &Signature::from_bytes(&signature_bytes),
        )
        .context("manifest signature verification failed")
}

fn verify_payload(manifest: &toml::Value, entries: &[MpkgEntry]) -> Result<()> {
    let package_kind = manifest
        .get("package")
        .and_then(|package| package.get("kind"))
        .and_then(toml::Value::as_str);
    if !matches!(package_kind, None | Some("binary") | Some("application")) {
        bail!("unsupported package kind");
    }
    let files = manifest
        .get("file")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| anyhow!("manifest must contain at least one [[file]]"))?;
    if files.is_empty() {
        bail!("manifest must contain at least one [[file]]");
    }
    let mut expected_paths = BTreeSet::new();
    for file in files {
        let path = file
            .get("path")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| anyhow!("file.path is missing"))?;
        let payload_path = manifest_payload_path(package_kind, path)?;
        if !expected_paths.insert(payload_path.clone()) {
            bail!("manifest contains duplicate payload path: {payload_path}");
        }
        let payload = entry(entries, &payload_path)?;
        let size = file
            .get("size")
            .and_then(toml::Value::as_integer)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| anyhow!("file.size is invalid"))?;
        if payload.data.len() as u64 != size {
            bail!("payload size mismatch: {payload_path}");
        }
        let digest = file
            .get("digest")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| anyhow!("file.digest is missing"))?;
        let expected_digest = decode_sha256(digest)?;
        if Sha256::digest(&payload.data).as_slice() != expected_digest {
            bail!("payload digest mismatch: {payload_path}");
        }
    }
    for payload in entries
        .iter()
        .filter(|entry| entry.path.starts_with("payload/"))
    {
        if !expected_paths.contains(&payload.path) {
            bail!("manifest does not declare payload: {}", payload.path);
        }
    }
    Ok(())
}

fn validate_manifest_shape(manifest: &toml::Value) -> Result<()> {
    let package = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| anyhow!("manifest is missing [package]"))?;
    let package_id = required_non_empty_string(package, "id", "package.id")?;
    if !is_valid_package_id(package_id) {
        bail!("package.id contains invalid characters");
    }
    required_non_empty_string(package, "name", "package.name")?;
    required_non_empty_string(package, "version", "package.version")?;

    if let Some(kind) = package.get("kind").and_then(toml::Value::as_str) {
        if !matches!(kind, "binary" | "application") {
            bail!("unsupported package kind");
        }
    }

    validate_manifest_binaries(manifest)?;
    validate_manifest_files(manifest)?;
    validate_install_targets(manifest)?;
    Ok(())
}

fn validate_manifest_binaries(manifest: &toml::Value) -> Result<()> {
    let Some(binaries) = manifest.get("binary").and_then(toml::Value::as_array) else {
        return Ok(());
    };
    let mut paths = BTreeSet::new();
    for binary in binaries {
        let table = binary
            .as_table()
            .ok_or_else(|| anyhow!("binary entry must be a table"))?;
        let path = required_non_empty_string(table, "path", "binary.path")?;
        if !paths.insert(path.to_string()) {
            bail!("manifest contains duplicate binary path: {path}");
        }
        if let Some(requires) = table.get("requires") {
            let requires = requires
                .as_array()
                .ok_or_else(|| anyhow!("binary.requires must be an array"))?;
            for capability in requires {
                if capability.as_str().is_none() {
                    bail!("binary.requires must contain strings");
                }
            }
        }
    }
    Ok(())
}

fn validate_manifest_files(manifest: &toml::Value) -> Result<()> {
    let Some(files) = manifest.get("file").and_then(toml::Value::as_array) else {
        return Ok(());
    };
    let mut ids = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for file in files {
        let table = file
            .as_table()
            .ok_or_else(|| anyhow!("file entry must be a table"))?;
        let id = required_non_empty_string(table, "id", "file.id")?;
        if !ids.insert(id.to_string()) {
            bail!("manifest contains duplicate file id: {id}");
        }
        let path = required_non_empty_string(table, "path", "file.path")?;
        if !paths.insert(path.to_string()) {
            bail!("manifest contains duplicate file path: {path}");
        }
        required_non_empty_string(table, "digest", "file.digest")?;
        manifest_file_mode(table)?;
    }
    Ok(())
}

fn validate_install_targets(manifest: &toml::Value) -> Result<()> {
    let package_kind = manifest
        .get("package")
        .and_then(|package| package.get("kind"))
        .and_then(toml::Value::as_str);
    let package_name = manifest
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| anyhow!("package.name is missing"))?;
    let Some(files) = manifest.get("file").and_then(toml::Value::as_array) else {
        return Ok(());
    };
    for file in files {
        let table = file
            .as_table()
            .ok_or_else(|| anyhow!("file entry must be a table"))?;
        let path = required_non_empty_string(table, "path", "file.path")?;
        let target = manifest_target_path(package_kind, package_name, path)?;
        if !is_valid_abs_path(&target) {
            bail!("file target path is invalid: {target}");
        }
        if !is_allowed_install_target(package_kind, &target) {
            bail!("file target path is outside installable roots: {target}");
        }
        let mode = manifest_file_mode(table)?;
        if mode & !0o777 != 0 {
            bail!("file.mode exceeds installable permission bits");
        }
    }
    Ok(())
}

fn required_non_empty_string<'a>(
    table: &'a toml::map::Map<String, toml::Value>,
    key: &str,
    name: &str,
) -> Result<&'a str> {
    let value = table
        .get(key)
        .and_then(toml::Value::as_str)
        .ok_or_else(|| anyhow!("{name} is missing"))?;
    if value.is_empty() {
        bail!("{name} is empty");
    }
    Ok(value)
}

fn manifest_file_mode(table: &toml::map::Map<String, toml::Value>) -> Result<u32> {
    manifest_file_mode_value(table.get("mode"))
}

fn manifest_file_mode_value(value: Option<&toml::Value>) -> Result<u32> {
    let mode = match value {
        Some(toml::Value::String(value)) if !value.is_empty() && value.len() <= 4 => {
            u32::from_str_radix(value, 8).context("file.mode is invalid")?
        }
        Some(toml::Value::Integer(value)) if *value >= 0 => {
            let value = value.to_string();
            if value.is_empty() || value.len() > 4 {
                bail!("file.mode is invalid");
            }
            u32::from_str_radix(&value, 8).context("file.mode is invalid")?
        }
        _ => bail!("file.mode is invalid"),
    };
    if mode == 0 {
        bail!("file.mode is invalid");
    }
    Ok(mode)
}

fn validate_certificate_for_manifest(
    certificate: &DeveloperCertificate,
    manifest: &toml::Value,
    package_id: &str,
    unix_time: Option<u64>,
) -> Result<()> {
    if !certificate
        .package_id_scopes
        .iter()
        .any(|scope| scope.matches(package_id))
    {
        bail!("Package ID is outside Certificate scope");
    }
    validate_certificate_capabilities_for_manifest(certificate, manifest)?;
    let now = match unix_time {
        Some(value) => value,
        None => current_unix_time()?,
    };
    if now < certificate.not_before || now >= certificate.not_after {
        bail!("Certificate is expired or not yet valid");
    }
    Ok(())
}

fn validate_certificate_capabilities_for_manifest(
    certificate: &DeveloperCertificate,
    manifest: &toml::Value,
) -> Result<()> {
    for capability in required_capabilities(manifest)? {
        if !certificate
            .allowed_capabilities
            .iter()
            .any(|allowed| allowed == &capability)
        {
            bail!("Capability is not allowed by Certificate: {capability}");
        }
    }
    Ok(())
}

fn required_capabilities(manifest: &toml::Value) -> Result<Vec<String>> {
    let mut capabilities = Vec::new();
    let Some(binaries) = manifest.get("binary").and_then(toml::Value::as_array) else {
        return Ok(capabilities);
    };
    for binary in binaries {
        let Some(requires) = binary.get("requires").and_then(toml::Value::as_array) else {
            continue;
        };
        for capability in requires {
            let value = capability
                .as_str()
                .ok_or_else(|| anyhow!("binary.requires must contain strings"))?;
            capabilities.push(value.to_string());
        }
    }
    Ok(capabilities)
}

fn current_unix_time() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system time is before UNIX_EPOCH")?
        .as_secs())
}

fn manifest_payload_path(package_kind: Option<&str>, path: &str) -> Result<String> {
    if path.starts_with('/') {
        return Ok(format!("payload/root{path}"));
    }
    let relative = path
        .strip_prefix("$/")
        .ok_or_else(|| anyhow!("file.path must be absolute or start with $/"))?;
    match package_kind {
        Some("application") => Ok(format!("payload/bundle/{relative}")),
        None | Some("binary") => Ok(format!("payload/root/bin/{relative}")),
        _ => bail!("unsupported package kind"),
    }
}

fn manifest_target_path(
    package_kind: Option<&str>,
    package_name: &str,
    path: &str,
) -> Result<String> {
    if path.starts_with('/') {
        return Ok(path.to_string());
    }
    let relative = path
        .strip_prefix("$/")
        .ok_or_else(|| anyhow!("file.path must be absolute or start with $/"))?;
    match package_kind {
        Some("application") => Ok(join_path(
            &format!("/applications/{package_name}.app"),
            relative,
        )),
        None | Some("binary") => Ok(join_path("/bin", relative)),
        _ => bail!("unsupported package kind"),
    }
}

fn join_path(prefix: &str, suffix: &str) -> String {
    if prefix.is_empty() {
        return suffix.to_string();
    }
    if suffix.is_empty() {
        return prefix.to_string();
    }
    format!(
        "{}/{}",
        prefix.trim_end_matches('/'),
        suffix.trim_start_matches('/')
    )
}

fn is_valid_abs_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains('\\')
        && !path.contains('\0')
        && !path.contains("//")
        && !path.ends_with('/')
        && path[1..]
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn is_allowed_install_target(package_kind: Option<&str>, target: &str) -> bool {
    target.starts_with("/bin/")
        || target.starts_with("/libraries/")
        || target.starts_with("/binary/services/")
        || target.starts_with("/binary/resources/")
        || target.starts_with("/system/services/")
        || (target.starts_with("/applications/") && package_kind == Some("application"))
}

fn package_id(manifest: &toml::Value) -> Result<&str> {
    manifest
        .get("package")
        .and_then(|package| package.get("id"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| anyhow!("manifest is missing package.id"))
}

fn entry<'a>(entries: &'a [MpkgEntry], path: &str) -> Result<&'a MpkgEntry> {
    entries
        .iter()
        .find(|entry| entry.path == path)
        .ok_or_else(|| anyhow!("MPKG is missing {path}"))
}

fn manifest_signing_message(manifest: &[u8]) -> Vec<u8> {
    let digest = Sha256::digest(manifest);
    let mut message = Vec::with_capacity(MANIFEST_DOMAIN.len() + digest.len());
    message.extend_from_slice(MANIFEST_DOMAIN);
    message.extend_from_slice(&digest);
    message
}

fn decode_sha256(value: &str) -> Result<[u8; 32]> {
    let hex = value
        .strip_prefix("sha256:")
        .ok_or_else(|| anyhow!("file.digest must use sha256:"))?;
    if hex.len() != 64 {
        bail!("SHA-256 digest must contain 64 hexadecimal characters");
    }
    let mut output = [0u8; 32];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .context("SHA-256 digest contains non-hexadecimal characters")?;
    }
    Ok(output)
}

fn normalize_path(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| anyhow!("MPKG path is not UTF-8"))?
        .to_string();
    if value.is_empty()
        || value.starts_with('/')
        || value.contains("//")
        || value.contains('\\')
        || value
            .split('/')
            .any(|segment| segment == "." || segment == "..")
    {
        bail!("invalid MPKG path: {value}");
    }
    Ok(value)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use mochios_certificate::{key_id, PackageIdScope, KEY_USAGE_PACKAGE_SIGNING, SIGNATURE_LEN};

    #[test]
    fn manifest_path_mapping_matches_mpkg_v1() {
        assert_eq!(
            manifest_payload_path(Some("binary"), "/bin/example").unwrap(),
            "payload/root/bin/example"
        );
        assert_eq!(
            manifest_payload_path(Some("application"), "$/entry.elf").unwrap(),
            "payload/bundle/entry.elf"
        );
    }

    #[test]
    fn rejects_glob_like_and_parent_paths() {
        assert!(normalize_path(Path::new("payload/../manifest.toml")).is_err());
        assert!(normalize_path(Path::new("/manifest.toml")).is_err());
        assert!(normalize_path(Path::new("./manifest.toml")).is_err());
    }

    #[test]
    fn certificate_request_extracts_package_and_capability_union() {
        let temporary = tempfile::tempdir().unwrap();
        let (_, developer_public) = crypto::generate_keypair();
        let unsigned = write_test_unsigned_package(
            temporary.path(),
            &["window.create", "process.spawn", "window.create"],
        );

        let request = certificate_request(
            &unsigned,
            "019f9e5ac6687902b0e72fe53abfbef1",
            &developer_public,
        )
        .unwrap();

        assert_eq!(request.developer_id, "019f9e5ac6687902b0e72fe53abfbef1");
        assert_eq!(request.package_id, "org.example.application");
        assert_eq!(
            request.subject_public_key,
            crypto::public_key_to_base64(&developer_public)
        );
        assert_eq!(
            request.capabilities,
            vec!["process.spawn".to_string(), "window.create".to_string()]
        );
    }

    #[test]
    fn manifest_shape_rejects_signature_service_incompatible_metadata() {
        let invalid_package_id = toml::from_str(
            r#"
            [package]
            id = "Org.Example.Application"
            name = "Example"
            version = "0.1.0"
            "#,
        )
        .unwrap();
        assert!(validate_manifest_shape(&invalid_package_id).is_err());

        let duplicate_file_id = toml::from_str(
            r#"
            [package]
            id = "org.example.application"
            name = "Example"
            version = "0.1.0"

            [[file]]
            id = "entry"
            path = "$/entry.elf"
            digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            mode = "0755"

            [[file]]
            id = "entry"
            path = "$/other.elf"
            digest = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            mode = "0755"
            "#,
        )
        .unwrap();
        assert!(validate_manifest_shape(&duplicate_file_id).is_err());

        let empty_binary_path = toml::from_str(
            r#"
            [package]
            id = "org.example.application"
            name = "Example"
            version = "0.1.0"

            [[binary]]
            path = ""
            requires = []
            "#,
        )
        .unwrap();
        assert!(validate_manifest_shape(&empty_binary_path).is_err());

        let disallowed_application_target = toml::from_str(
            r#"
            [package]
            id = "org.example.application"
            name = "Example"
            version = "0.1.0"
            kind = "application"

            [[file]]
            id = "entry"
            path = "/etc/entry.elf"
            digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            mode = "0755"
            "#,
        )
        .unwrap();
        assert!(validate_manifest_shape(&disallowed_application_target).is_err());

        let invalid_application_target = toml::from_str(
            r#"
            [package]
            id = "org.example.application"
            name = "Example"
            version = "0.1.0"
            kind = "application"

            [[file]]
            id = "entry"
            path = "$/../entry.elf"
            digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            mode = "0755"
            "#,
        )
        .unwrap();
        assert!(validate_manifest_shape(&invalid_application_target).is_err());

        let invalid_mode_bits = toml::from_str(
            r#"
            [package]
            id = "org.example.application"
            name = "Example"
            version = "0.1.0"
            kind = "application"

            [[file]]
            id = "entry"
            path = "$/entry.elf"
            digest = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            mode = "1777"
            "#,
        )
        .unwrap();
        assert!(validate_manifest_shape(&invalid_mode_bits).is_err());
    }

    #[test]
    fn manifest_mode_matches_signature_service_octal_parser() {
        let string_mode = toml::Value::String("0755".to_string());
        assert_eq!(manifest_file_mode_value(Some(&string_mode)).unwrap(), 0o755);

        let integer_mode = toml::Value::Integer(755);
        assert_eq!(
            manifest_file_mode_value(Some(&integer_mode)).unwrap(),
            0o755
        );

        let ambiguous_decimal = toml::Value::Integer(420);
        assert_eq!(
            manifest_file_mode_value(Some(&ambiguous_decimal)).unwrap(),
            0o420
        );
    }

    #[test]
    fn signs_and_verifies_mpkg_manifest_with_certificate() {
        let temporary = tempfile::tempdir().unwrap();
        let unsigned = write_test_unsigned_package(temporary.path(), &["window.create"]);

        let (root_key, root_public) = crypto::generate_keypair();
        let (developer_key, developer_public) = crypto::generate_keypair();
        let root_public_path = temporary.path().join("root.pub");
        let developer_key_path = temporary.path().join("application.key");
        let certificate_path = temporary.path().join("developer.cert");
        crypto::write_public_key(&root_public_path, &root_public).unwrap();
        crypto::write_private_key(&developer_key_path, &developer_key).unwrap();
        write_test_certificate(
            &certificate_path,
            &root_key,
            TestCertificateSpec {
                root_public_bytes: &root_public.to_bytes(),
                developer_public_bytes: &developer_public.to_bytes(),
                package_id_scopes: vec![PackageIdScope::exact("org.example.application")],
                allowed_capabilities: vec!["window.create".to_string()],
                not_before: 1_700_000_000,
                not_after: 1_900_000_000,
            },
        );

        let signed = temporary.path().join("signed.mpkg");
        sign(PackageSignArgs {
            package: unsigned.clone(),
            certificate: certificate_path.clone(),
            key: developer_key_path.clone(),
            output: Some(signed.clone()),
            unix_time: Some(1_800_000_000),
            replace_signature: false,
        })
        .unwrap();

        assert!(sign(PackageSignArgs {
            package: signed.clone(),
            certificate: certificate_path,
            key: developer_key_path,
            output: Some(temporary.path().join("resigned.mpkg")),
            unix_time: Some(1_800_000_000),
            replace_signature: false,
        })
        .is_err());

        verify(PackageVerifyArgs {
            package: signed.clone(),
            root_public_key: root_public_path,
            unix_time: 1_800_000_000,
        })
        .unwrap();

        let no_capability_certificate = temporary.path().join("no-capability.cert");
        write_test_certificate(
            &no_capability_certificate,
            &root_key,
            TestCertificateSpec {
                root_public_bytes: &root_public.to_bytes(),
                developer_public_bytes: &developer_public.to_bytes(),
                package_id_scopes: vec![PackageIdScope::exact("org.example.application")],
                allowed_capabilities: Vec::new(),
                not_before: 1_700_000_000,
                not_after: 1_900_000_000,
            },
        );
        let mut no_capability_entries = read_mpkg(&signed).unwrap();
        entry_mut(&mut no_capability_entries, CERTIFICATE_PATH)
            .unwrap()
            .data = fs::read(no_capability_certificate).unwrap();
        let no_capability = temporary.path().join("no-capability.mpkg");
        write_mpkg(&no_capability, &no_capability_entries).unwrap();
        assert!(verify(PackageVerifyArgs {
            package: no_capability,
            root_public_key: temporary.path().join("root.pub"),
            unix_time: 1_800_000_000,
        })
        .is_err());

        let mut missing_certificate_entries = read_mpkg(&signed).unwrap();
        missing_certificate_entries.retain(|entry| entry.path != CERTIFICATE_PATH);
        let missing_certificate = temporary.path().join("missing-certificate.mpkg");
        write_mpkg(&missing_certificate, &missing_certificate_entries).unwrap();
        assert!(verify(PackageVerifyArgs {
            package: missing_certificate,
            root_public_key: temporary.path().join("root.pub"),
            unix_time: 1_800_000_000,
        })
        .is_err());

        let mut missing_signature_entries = read_mpkg(&signed).unwrap();
        missing_signature_entries.retain(|entry| entry.path != MANIFEST_SIGNATURE_PATH);
        let missing_signature = temporary.path().join("missing-signature.mpkg");
        write_mpkg(&missing_signature, &missing_signature_entries).unwrap();
        assert!(verify(PackageVerifyArgs {
            package: missing_signature,
            root_public_key: temporary.path().join("root.pub"),
            unix_time: 1_800_000_000,
        })
        .is_err());

        let mut signature_tampered_entries = read_mpkg(&signed).unwrap();
        entry_mut(&mut signature_tampered_entries, MANIFEST_SIGNATURE_PATH)
            .unwrap()
            .data[0] ^= 1;
        let signature_tampered = temporary.path().join("signature-tampered.mpkg");
        write_mpkg(&signature_tampered, &signature_tampered_entries).unwrap();
        assert!(verify(PackageVerifyArgs {
            package: signature_tampered,
            root_public_key: temporary.path().join("root.pub"),
            unix_time: 1_800_000_000,
        })
        .is_err());

        let mut tampered_entries = read_mpkg(&signed).unwrap();
        entry_mut(&mut tampered_entries, "payload/bundle/entry.elf")
            .unwrap()
            .data = b"tampered".to_vec();
        let tampered = temporary.path().join("tampered.mpkg");
        write_mpkg(&tampered, &tampered_entries).unwrap();
        assert!(verify(PackageVerifyArgs {
            package: tampered,
            root_public_key: temporary.path().join("root.pub"),
            unix_time: 1_800_000_000,
        })
        .is_err());

        let mut undeclared_entries = read_mpkg(&signed).unwrap();
        undeclared_entries.push(MpkgEntry {
            path: "payload/bundle/extra.dat".to_string(),
            data: b"extra".to_vec(),
            mode: 0o644,
        });
        let undeclared = temporary.path().join("undeclared.mpkg");
        write_mpkg(&undeclared, &undeclared_entries).unwrap();
        assert!(verify(PackageVerifyArgs {
            package: undeclared,
            root_public_key: temporary.path().join("root.pub"),
            unix_time: 1_800_000_000,
        })
        .is_err());
    }

    #[test]
    fn signing_rejects_key_scope_capability_and_time_mismatches() {
        let temporary = tempfile::tempdir().unwrap();
        let unsigned = write_test_unsigned_package(temporary.path(), &["window.create"]);
        let (root_key, root_public) = crypto::generate_keypair();
        let (developer_key, developer_public) = crypto::generate_keypair();
        let (other_key, other_public) = crypto::generate_keypair();
        let developer_key_path = temporary.path().join("application.key");
        let other_key_path = temporary.path().join("other.key");
        crypto::write_private_key(&developer_key_path, &developer_key).unwrap();
        crypto::write_private_key(&other_key_path, &other_key).unwrap();

        let valid_certificate = temporary.path().join("valid.cert");
        write_test_certificate(
            &valid_certificate,
            &root_key,
            TestCertificateSpec {
                root_public_bytes: &root_public.to_bytes(),
                developer_public_bytes: &developer_public.to_bytes(),
                package_id_scopes: vec![PackageIdScope::exact("org.example.application")],
                allowed_capabilities: vec!["window.create".to_string()],
                not_before: 1_700_000_000,
                not_after: 1_900_000_000,
            },
        );
        assert!(sign(PackageSignArgs {
            package: unsigned.clone(),
            certificate: valid_certificate,
            key: other_key_path,
            output: Some(temporary.path().join("key-mismatch.mpkg")),
            unix_time: Some(1_800_000_000),
            replace_signature: false,
        })
        .is_err());

        let scope_certificate = temporary.path().join("scope.cert");
        write_test_certificate(
            &scope_certificate,
            &root_key,
            TestCertificateSpec {
                root_public_bytes: &root_public.to_bytes(),
                developer_public_bytes: &developer_public.to_bytes(),
                package_id_scopes: vec![PackageIdScope::exact("org.example.other")],
                allowed_capabilities: vec!["window.create".to_string()],
                not_before: 1_700_000_000,
                not_after: 1_900_000_000,
            },
        );
        assert!(sign(PackageSignArgs {
            package: unsigned.clone(),
            certificate: scope_certificate,
            key: developer_key_path.clone(),
            output: Some(temporary.path().join("scope-mismatch.mpkg")),
            unix_time: Some(1_800_000_000),
            replace_signature: false,
        })
        .is_err());

        let capability_certificate = temporary.path().join("capability.cert");
        write_test_certificate(
            &capability_certificate,
            &root_key,
            TestCertificateSpec {
                root_public_bytes: &root_public.to_bytes(),
                developer_public_bytes: &developer_public.to_bytes(),
                package_id_scopes: vec![PackageIdScope::exact("org.example.application")],
                allowed_capabilities: Vec::new(),
                not_before: 1_700_000_000,
                not_after: 1_900_000_000,
            },
        );
        assert!(sign(PackageSignArgs {
            package: unsigned.clone(),
            certificate: capability_certificate,
            key: developer_key_path.clone(),
            output: Some(temporary.path().join("capability-mismatch.mpkg")),
            unix_time: Some(1_800_000_000),
            replace_signature: false,
        })
        .is_err());

        let expired_certificate = temporary.path().join("expired.cert");
        write_test_certificate(
            &expired_certificate,
            &root_key,
            TestCertificateSpec {
                root_public_bytes: &root_public.to_bytes(),
                developer_public_bytes: &developer_public.to_bytes(),
                package_id_scopes: vec![PackageIdScope::exact("org.example.application")],
                allowed_capabilities: vec!["window.create".to_string()],
                not_before: 1_700_000_000,
                not_after: 1_750_000_000,
            },
        );
        assert!(sign(PackageSignArgs {
            package: unsigned,
            certificate: expired_certificate,
            key: developer_key_path,
            output: Some(temporary.path().join("expired.mpkg")),
            unix_time: Some(1_800_000_000),
            replace_signature: false,
        })
        .is_err());

        assert_ne!(developer_public.to_bytes(), other_public.to_bytes());
    }

    #[test]
    fn parser_rejects_unknown_top_level_entries() {
        let temporary = tempfile::tempdir().unwrap();
        let invalid = temporary.path().join("invalid.mpkg");
        write_mpkg(
            &invalid,
            &[
                MpkgEntry {
                    path: MANIFEST_PATH.to_string(),
                    data: b"format = 1\n".to_vec(),
                    mode: 0o644,
                },
                MpkgEntry {
                    path: "metadata/extra.toml".to_string(),
                    data: b"extra".to_vec(),
                    mode: 0o644,
                },
            ],
        )
        .unwrap();

        assert!(read_mpkg(&invalid).is_err());
    }

    #[test]
    fn parser_rejects_signature_chain_directory_entries() {
        let temporary = tempfile::tempdir().unwrap();
        let invalid = temporary.path().join("signature-chain-directory.mpkg");
        write_raw_mpkg(
            &invalid,
            &[
                RawTarEntry {
                    path: MANIFEST_PATH,
                    kind: b'0',
                    data: b"format = 1\n",
                    magic: b"ustar\0",
                    version: b"00",
                },
                RawTarEntry {
                    path: "signatures/chain",
                    kind: b'5',
                    data: b"",
                    magic: b"ustar\0",
                    version: b"00",
                },
            ],
        );

        assert!(read_mpkg(&invalid).is_err());
    }

    #[test]
    fn package_read_rejects_appstore_reviewer_size_limit() {
        let temporary = tempfile::tempdir().unwrap();
        let oversized = temporary.path().join("oversized.mpkg");
        fs::File::create(&oversized)
            .unwrap()
            .set_len(MAX_PACKAGE_LEN + 1)
            .unwrap();

        assert!(read_mpkg(&oversized).is_err());
        let (_, developer_public) = crypto::generate_keypair();
        assert!(certificate_request(
            &oversized,
            "019f9e5ac6687902b0e72fe53abfbef1",
            &developer_public
        )
        .is_err());
        assert!(verify(PackageVerifyArgs {
            package: oversized,
            root_public_key: temporary.path().join("root.pub"),
            unix_time: 1_800_000_000,
        })
        .is_err());
    }

    #[test]
    fn parser_rejects_appstore_reviewer_entry_and_metadata_limits() {
        let temporary = tempfile::tempdir().unwrap();
        let too_many_entries = temporary.path().join("too-many-entries.mpkg");
        let mut tar_bytes = Vec::new();
        for index in 0..=MAX_ENTRIES {
            let path = format!("payload/bundle/{index}");
            append_raw_tar_entry(
                &mut tar_bytes,
                &RawTarEntry {
                    path: &path,
                    kind: b'5',
                    data: b"",
                    magic: b"ustar\0",
                    version: b"00",
                },
            );
        }
        write_raw_mpkg_bytes(&too_many_entries, tar_bytes);
        assert!(read_mpkg(&too_many_entries).is_err());

        let oversized_manifest = vec![b'x'; MAX_METADATA_LEN + 1];
        let too_much_metadata = temporary.path().join("too-much-metadata.mpkg");
        write_raw_mpkg(
            &too_much_metadata,
            &[RawTarEntry {
                path: MANIFEST_PATH,
                kind: b'0',
                data: &oversized_manifest,
                magic: b"ustar\0",
                version: b"00",
            }],
        );
        assert!(read_mpkg(&too_much_metadata).is_err());
    }

    #[test]
    fn parser_rejects_dot_segment_entries() {
        let temporary = tempfile::tempdir().unwrap();
        let invalid = temporary.path().join("dot-segment.mpkg");
        write_raw_mpkg(
            &invalid,
            &[RawTarEntry {
                path: "./manifest.toml",
                kind: b'0',
                data: b"format = 1\n",
                magic: b"ustar\0",
                version: b"00",
            }],
        );

        assert!(read_mpkg(&invalid).is_err());
    }

    #[test]
    fn parser_rejects_duplicate_entries() {
        let temporary = tempfile::tempdir().unwrap();
        let invalid = temporary.path().join("duplicate.mpkg");
        write_mpkg(
            &invalid,
            &[
                MpkgEntry {
                    path: MANIFEST_PATH.to_string(),
                    data: b"format = 1\n".to_vec(),
                    mode: 0o644,
                },
                MpkgEntry {
                    path: "payload/bundle/entry.elf".to_string(),
                    data: b"first".to_vec(),
                    mode: 0o644,
                },
                MpkgEntry {
                    path: "payload/bundle/entry.elf".to_string(),
                    data: b"second".to_vec(),
                    mode: 0o644,
                },
            ],
        )
        .unwrap();

        assert!(read_mpkg(&invalid).is_err());
    }

    #[test]
    fn parser_rejects_pax_and_gnu_extension_entries() {
        let temporary = tempfile::tempdir().unwrap();
        let pax = temporary.path().join("pax.mpkg");
        write_raw_mpkg(
            &pax,
            &[
                RawTarEntry {
                    path: "pax-header",
                    kind: b'x',
                    data: b"14 path=manifest.toml\n",
                    magic: b"ustar\0",
                    version: b"00",
                },
                RawTarEntry {
                    path: MANIFEST_PATH,
                    kind: b'0',
                    data: b"format = 1\n",
                    magic: b"ustar\0",
                    version: b"00",
                },
            ],
        );
        assert!(read_mpkg(&pax).is_err());

        let gnu_long = temporary.path().join("gnu-long.mpkg");
        write_raw_mpkg(
            &gnu_long,
            &[
                RawTarEntry {
                    path: "././@LongLink",
                    kind: b'L',
                    data: b"manifest.toml\0",
                    magic: b"ustar\0",
                    version: b"00",
                },
                RawTarEntry {
                    path: MANIFEST_PATH,
                    kind: b'0',
                    data: b"format = 1\n",
                    magic: b"ustar\0",
                    version: b"00",
                },
            ],
        );
        assert!(read_mpkg(&gnu_long).is_err());
    }

    #[test]
    fn parser_rejects_gnu_tar_headers() {
        let temporary = tempfile::tempdir().unwrap();
        let invalid = temporary.path().join("gnu.mpkg");
        write_raw_mpkg(
            &invalid,
            &[RawTarEntry {
                path: MANIFEST_PATH,
                kind: b'0',
                data: b"format = 1\n",
                magic: b"ustar ",
                version: b" \0",
            }],
        );

        assert!(read_mpkg(&invalid).is_err());
    }

    #[test]
    fn parser_rejects_bad_tar_checksum() {
        let temporary = tempfile::tempdir().unwrap();
        let invalid = temporary.path().join("bad-checksum.mpkg");
        write_raw_mpkg(
            &invalid,
            &[RawTarEntry {
                path: MANIFEST_PATH,
                kind: b'0',
                data: b"format = 1\n",
                magic: b"ustar\0",
                version: b"00",
            }],
        );

        let mut bytes = fs::read(&invalid).unwrap();
        bytes[MPKG_HEADER_LEN] ^= 1;
        fs::write(&invalid, bytes).unwrap();

        assert!(read_mpkg(&invalid).is_err());
    }

    struct TestCertificateSpec<'a> {
        root_public_bytes: &'a [u8; 32],
        developer_public_bytes: &'a [u8; 32],
        package_id_scopes: Vec<PackageIdScope>,
        allowed_capabilities: Vec<String>,
        not_before: u64,
        not_after: u64,
    }

    fn write_test_certificate(
        path: &Path,
        root_key: &ed25519_dalek::SigningKey,
        spec: TestCertificateSpec<'_>,
    ) {
        let mut certificate = DeveloperCertificate {
            serial_number: 1,
            issuer_key_id: key_id(spec.root_public_bytes),
            developer_id: "019f9e5ac6687902b0e72fe53abfbef1".to_string(),
            subject_key_id: key_id(spec.developer_public_bytes),
            subject_public_key: *spec.developer_public_bytes,
            not_before: spec.not_before,
            not_after: spec.not_after,
            key_usage: KEY_USAGE_PACKAGE_SIGNING,
            package_id_scopes: spec.package_id_scopes,
            allowed_capabilities: spec.allowed_capabilities,
            signature: [0; SIGNATURE_LEN],
        };
        certificate.signature = root_key
            .sign(&certificate.signing_message().unwrap())
            .to_bytes();
        let mut certificate_bytes = vec![0; certificate.encoded_len().unwrap()];
        certificate.encode(&mut certificate_bytes).unwrap();
        fs::write(path, certificate_bytes).unwrap();
    }

    fn write_test_unsigned_package(directory: &Path, capabilities: &[&str]) -> std::path::PathBuf {
        let payload = b"elf";
        let requires = capabilities
            .iter()
            .map(|capability| format!("\"{capability}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let manifest = format!(
            "format = 1\n\n[package]\nid = \"org.example.application\"\nname = \"Example\"\nversion = \"0.1.0\"\nkind = \"application\"\n\n[[file]]\nid = \"entry\"\npath = \"$/entry.elf\"\ndigest = \"sha256:{}\"\nsize = {}\nmode = \"0755\"\n\n[[binary]]\npath = \"/applications/Example.app/entry.elf\"\nfile = \"entry\"\nkind = \"application\"\nrequires = [{}]\n",
            hex(&Sha256::digest(payload)),
            payload.len(),
            requires
        );
        let unsigned = directory.join("unsigned.mpkg");
        write_mpkg(
            &unsigned,
            &[
                MpkgEntry {
                    path: MANIFEST_PATH.to_string(),
                    data: manifest.into_bytes(),
                    mode: 0o644,
                },
                MpkgEntry {
                    path: "payload/bundle/entry.elf".to_string(),
                    data: payload.to_vec(),
                    mode: 0o644,
                },
            ],
        )
        .unwrap();
        unsigned
    }

    fn entry_mut<'a>(entries: &'a mut [MpkgEntry], path: &str) -> Result<&'a mut MpkgEntry> {
        entries
            .iter_mut()
            .find(|entry| entry.path == path)
            .ok_or_else(|| anyhow!("MPKG is missing {path}"))
    }

    struct RawTarEntry<'a> {
        path: &'a str,
        kind: u8,
        data: &'a [u8],
        magic: &'a [u8; 6],
        version: &'a [u8; 2],
    }

    fn write_raw_mpkg(path: &Path, entries: &[RawTarEntry<'_>]) {
        let mut tar_bytes = Vec::new();
        for entry in entries {
            append_raw_tar_entry(&mut tar_bytes, entry);
        }
        write_raw_mpkg_bytes(path, tar_bytes);
    }

    fn write_raw_mpkg_bytes(path: &Path, mut tar_bytes: Vec<u8>) {
        tar_bytes.extend_from_slice(&[0; 1024]);

        let mut header = [0u8; MPKG_HEADER_LEN];
        header[..4].copy_from_slice(MPKG_MAGIC);
        header[4..6].copy_from_slice(&1u16.to_le_bytes());
        header[8..10].copy_from_slice(&(MPKG_HEADER_LEN as u16).to_le_bytes());
        header[12..20].copy_from_slice(&(tar_bytes.len() as u64).to_le_bytes());
        let mut bytes = Vec::from(header);
        bytes.extend_from_slice(&tar_bytes);
        fs::write(path, bytes).unwrap();
    }

    fn append_raw_tar_entry(output: &mut Vec<u8>, entry: &RawTarEntry<'_>) {
        let mut header = [0u8; 512];
        let path = entry.path.as_bytes();
        header[..path.len()].copy_from_slice(path);
        write_octal_field(&mut header[100..108], 0o644);
        write_octal_field(&mut header[108..116], 0);
        write_octal_field(&mut header[116..124], 0);
        write_octal_field(&mut header[124..136], entry.data.len() as u64);
        write_octal_field(&mut header[136..148], 0);
        header[148..156].fill(b' ');
        header[156] = entry.kind;
        header[257..263].copy_from_slice(entry.magic);
        header[263..265].copy_from_slice(entry.version);
        let checksum = header.iter().map(|byte| u64::from(*byte)).sum::<u64>();
        write_octal_field(&mut header[148..156], checksum);
        output.extend_from_slice(&header);
        output.extend_from_slice(entry.data);
        let padding = (512 - entry.data.len() % 512) % 512;
        output.resize(output.len() + padding, 0);
    }

    fn write_octal_field(field: &mut [u8], value: u64) {
        field.fill(0);
        let digits = format!("{:0width$o}", value, width = field.len() - 1);
        field[..digits.len()].copy_from_slice(digits.as_bytes());
    }
}
