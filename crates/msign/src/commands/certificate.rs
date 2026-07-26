use std::fs;

use anyhow::{anyhow, bail, Context, Result};
use ed25519_dalek::Signer;
use mochios_certificate::{
    key_id, DeveloperCertificate, PackageIdScope, KEY_USAGE_PACKAGE_SIGNING, SIGNATURE_LEN,
};

use crate::cli::{CertificateInspectArgs, CertificateIssueArgs};
use crate::crypto;

pub fn issue(args: CertificateIssueArgs) -> Result<()> {
    let root_key = crypto::read_private_key(&args.root_key)?;
    let developer_key = crypto::read_private_key(&args.developer_key)?;
    let root_public_key = root_key.verifying_key().to_bytes();
    let subject_public_key = developer_key.verifying_key().to_bytes();
    let mut package_id_scopes = args
        .scopes
        .iter()
        .map(|value| parse_scope(value))
        .collect::<Result<Vec<_>>>()?;
    package_id_scopes.sort();
    let mut allowed_capabilities = args.capabilities;
    allowed_capabilities.sort();

    let mut certificate = DeveloperCertificate {
        serial_number: args.serial,
        issuer_key_id: key_id(&root_public_key),
        developer_id: args.developer_id,
        subject_key_id: key_id(&subject_public_key),
        subject_public_key,
        not_before: args.not_before,
        not_after: args.not_after,
        key_usage: KEY_USAGE_PACKAGE_SIGNING,
        package_id_scopes,
        allowed_capabilities,
        signature: [0; SIGNATURE_LEN],
    };
    certificate.validate().map_err(|error| anyhow!(error))?;
    let message = certificate
        .signing_message()
        .map_err(|error| anyhow!(error))?;
    certificate.signature = root_key.sign(&message).to_bytes();
    let length = certificate.encoded_len().map_err(|error| anyhow!(error))?;
    let mut encoded = vec![0; length];
    certificate
        .encode(&mut encoded)
        .map_err(|error| anyhow!(error))?;
    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&args.output, encoded)
        .with_context(|| format!("failed to write {}", args.output.display()))?;
    println!("issued: {}", args.output.display());
    println!("developer_id: {}", certificate.developer_id);
    println!("serial_number: {}", certificate.serial_number);
    println!("subject_key_id: {}", hex(&certificate.subject_key_id));
    Ok(())
}

pub fn inspect(args: CertificateInspectArgs) -> Result<()> {
    let bytes = fs::read(&args.certificate)
        .with_context(|| format!("failed to read {}", args.certificate.display()))?;
    let certificate = DeveloperCertificate::decode(&bytes).map_err(|error| anyhow!(error))?;
    println!("format_version: 1");
    println!("serial_number: {}", certificate.serial_number);
    println!("issuer_key_id: {}", hex(&certificate.issuer_key_id));
    println!("developer_id: {}", certificate.developer_id);
    println!("subject_key_id: {}", hex(&certificate.subject_key_id));
    println!(
        "subject_public_key: {}",
        hex(&certificate.subject_public_key)
    );
    println!("not_before: {}", certificate.not_before);
    println!("not_after: {}", certificate.not_after);
    println!("key_usage: {:#x}", certificate.key_usage);
    for scope in certificate.package_id_scopes {
        println!("package_scope: {:?}:{}", scope.kind, scope.package_id);
    }
    for capability in certificate.allowed_capabilities {
        println!("allowed_capability: {capability}");
    }
    println!("signature: {}", hex(&certificate.signature));
    Ok(())
}

fn parse_scope(value: &str) -> Result<PackageIdScope> {
    if let Some(package_id) = value.strip_prefix("exact:") {
        return Ok(PackageIdScope::exact(package_id));
    }
    if let Some(package_id) = value.strip_prefix("prefix:") {
        return Ok(PackageIdScope::prefix(package_id));
    }
    bail!("scope must be exact:PACKAGE_ID or prefix:PACKAGE_ID")
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
