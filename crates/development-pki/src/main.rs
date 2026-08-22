use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signer, SigningKey};
use mochios_certificate::{
    key_id as certificate_key_id, DeveloperCertificate, PackageIdScope,
    KEY_USAGE_PACKAGE_SIGNING, SIGNATURE_LEN,
};
use mochios_developer_ca_trust::{
    key_id, IssuerRecord, IssuerStatus, RevocationSnapshot, TrustSnapshot,
    UnsignedRevocationSnapshot, UnsignedTrustSnapshot,
};
use rand_core::OsRng;

const SIX_DAYS: u64 = 6 * 24 * 60 * 60;
const SEVEN_DAYS: u64 = 7 * 24 * 60 * 60;
const ONE_HUNDRED_EIGHTY_DAYS: u64 = 180 * 24 * 60 * 60;
const TEN_YEARS: u64 = 10 * 365 * 24 * 60 * 60;
const DEVELOPER_ID: &str = "019f9e5ac6687902b0e72fe53abfbef1";

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let command = args.next().context("usage: development-pki <rotate|refresh> FIXTURE_DIR [UNIX_TIME]")?;
    let directory = PathBuf::from(
        args.next()
            .context("usage: development-pki <rotate|refresh> FIXTURE_DIR [UNIX_TIME]")?,
    );
    let now = match args.next() {
        Some(value) => value
            .to_string_lossy()
            .parse::<u64>()
            .context("UNIX_TIME must be an unsigned integer")?,
        None => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock predates the Unix epoch")?
            .as_secs(),
    };
    if args.next().is_some() {
        bail!("too many arguments");
    }

    match command.to_string_lossy().as_ref() {
        "rotate" => rotate(&directory, now),
        "refresh" => refresh(&directory, now),
        _ => bail!("unknown command; expected rotate or refresh"),
    }
}

fn rotate(directory: &Path, now: u64) -> Result<()> {
    fs::create_dir_all(directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;
    write_private_key(&directory.join("root.key"), &SigningKey::generate(&mut OsRng))?;
    write_private_key(&directory.join("issuer.key"), &SigningKey::generate(&mut OsRng))?;
    write_private_key(
        &directory.join("developer.key"),
        &SigningKey::generate(&mut OsRng),
    )?;
    refresh(directory, now)
}

fn refresh(directory: &Path, now: u64) -> Result<()> {
    // A six-day bucket keeps builds reproducible within the bucket while ensuring
    // the protocol's seven-day revocation maximum never expires in a fresh build.
    let generated_at = now / SIX_DAYS * SIX_DAYS;
    let root = read_private_key(&directory.join("root.key"))?;
    let issuer = read_private_key(&directory.join("issuer.key"))?;
    let developer = read_private_key(&directory.join("developer.key"))?;
    let root_public = root.verifying_key().to_bytes();
    let issuer_public = issuer.verifying_key().to_bytes();
    let developer_public = developer.verifying_key().to_bytes();

    let trust = TrustSnapshot::issue(
        UnsignedTrustSnapshot {
            format_version: 1,
            snapshot_version: generated_at / SIX_DAYS + 1,
            generated_at,
            expires_at: generated_at + ONE_HUNDRED_EIGHTY_DAYS,
            root_key_id: key_id(&root_public),
            issuers: vec![IssuerRecord {
                issuer_key_id: key_id(&issuer_public),
                public_key: STANDARD.encode(issuer_public),
                status: IssuerStatus::Active,
                not_before: generated_at,
                not_after: generated_at + TEN_YEARS,
                allowed_key_usages: vec![
                    "developer-certificate-signing".to_string(),
                    "revocation-signing".to_string(),
                ],
            }],
            signature_algorithm: "ed25519".to_string(),
        },
        &root,
    )?;
    let revocations = RevocationSnapshot::issue(
        UnsignedRevocationSnapshot {
            format_version: 1,
            snapshot_version: generated_at / SIX_DAYS + 1,
            generated_at,
            expires_at: generated_at + SEVEN_DAYS,
            issuer_key_id: key_id(&issuer_public),
            revocations: vec![],
            signature_algorithm: "ed25519".to_string(),
        },
        &issuer,
    )?;

    let mut certificate = DeveloperCertificate {
        serial_number: 1,
        issuer_key_id: certificate_key_id(&issuer_public),
        developer_id: DEVELOPER_ID.to_string(),
        subject_key_id: certificate_key_id(&developer_public),
        subject_public_key: developer_public,
        not_before: generated_at,
        not_after: generated_at + TEN_YEARS,
        key_usage: KEY_USAGE_PACKAGE_SIGNING,
        package_id_scopes: vec![PackageIdScope::prefix("org.mochios")],
        allowed_capabilities: vec!["fs.read.all".to_string(), "ipc.client".to_string()],
        signature: [0; SIGNATURE_LEN],
    };
    certificate.allowed_capabilities.sort();
    let signing_message = certificate
        .signing_message()
        .map_err(|error| anyhow::anyhow!("certificate signing message failed: {error:?}"))?;
    certificate.signature = issuer
        .sign(&signing_message)
        .to_bytes();
    let certificate_len = certificate
        .encoded_len()
        .map_err(|error| anyhow::anyhow!("certificate length failed: {error:?}"))?;
    let mut encoded_certificate = vec![0; certificate_len];
    certificate
        .encode(&mut encoded_certificate)
        .map_err(|error| anyhow::anyhow!("certificate encoding failed: {error:?}"))?;

    write_text(&directory.join("root.pub"), &STANDARD.encode(root_public))?;
    write_json(&directory.join("trust-a.json"), &trust)?;
    write_json(&directory.join("revocations-a.json"), &revocations)?;
    write_text(
        &directory.join("developer.cert.b64"),
        &STANDARD.encode(encoded_certificate),
    )?;
    println!("root_public_key_hex={}", hex(&root_public));
    println!("generated_at={generated_at}");
    println!("revocations_expires_at={}", generated_at + SEVEN_DAYS);
    Ok(())
}

fn read_private_key(path: &Path) -> Result<SigningKey> {
    let encoded = fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let bytes = STANDARD
        .decode(encoded.trim())
        .with_context(|| format!("{} is not base64", path.display()))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("{} must contain 32 bytes", path.display()))?;
    Ok(SigningKey::from_bytes(&bytes))
}

fn write_private_key(path: &Path, key: &SigningKey) -> Result<()> {
    write_text(path, &STANDARD.encode(key.to_bytes()))
}

fn write_text(path: &Path, value: &str) -> Result<()> {
    write_if_changed(path, format!("{value}\n").as_bytes())
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    write_if_changed(path, &bytes)
}

fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<()> {
    if fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    fs::write(path, bytes).with_context(|| format!("failed to write {}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
