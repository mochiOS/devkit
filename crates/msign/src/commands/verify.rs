use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::header::ACCEPT;
use serde::Deserialize;

use crate::{
    cli::VerifyArgs,
    crypto,
    package,
    signature::{PackageSignature, SIGNATURE_ALGORITHM, SIGNATURE_VERSION},
};

const DEFAULT_API_BASE: &str = "https://api.mochios.org/v1";

#[derive(Debug, Deserialize)]
struct PublicKeyResponse {
    key: ApiPublicKey,
}

#[derive(Debug, Deserialize)]
struct ApiPublicKey {
    public_key: String,

    #[serde(rename = "fingerprint")]
    _fingerprint: String,

    revoked_at: Option<String>,
}

pub fn run(args: VerifyArgs) -> Result<()> {
    let signature = package::read_signature(&args.package)
        .with_context(|| format!("failed to read signature from {}", args.package.display()))?;

    if signature.version != SIGNATURE_VERSION {
        bail!("unsupported signature version: {}", signature.version);
    }

    if signature.algorithm != SIGNATURE_ALGORITHM {
        bail!("unsupported signature algorithm: {}", signature.algorithm);
    }

    let actual_hash = package::calculate_package_hash(&args.package)?;

    if actual_hash != signature.package_hash {
        bail!("package hash mismatch");
    }

    if !args.local {
        verify_registered_public_key(&signature.public_key, args.api_base)?;
    }

    let public_key = match &args.pubkey {
        Some(path) => crypto::read_public_key(path)
            .with_context(|| format!("failed to read public key from {}", path.display()))?,
        None => crypto::public_key_from_base64(&signature.public_key)?,
    };

    let message = PackageSignature::signing_message(
        &signature.package_hash,
        &signature.key_id,
        &signature.public_key,
    );

    crypto::verify(&public_key, &message, &signature.signature)?;

    println!("verified: {}", args.package.display());
    println!("key_id: {}", signature.key_id);
    println!("public_key: {:?}", signature.public_key);

    if args.local {
        println!("mode: local");
    } else {
        println!("mode: From signature server");
    }

    Ok(())
}

fn verify_registered_public_key(
    public_key: &str,
    api_base: Option<String>,
) -> Result<()> {
    let key = fetch_registered_public_key(public_key, api_base)?;

    if public_key_to_path_segment(&key.public_key) != public_key_to_path_segment(public_key) {
        bail!("public key mismatch: API returned a different public key");
    }

    if key.revoked_at.is_some() {
        bail!("public key has been revoked");
    }

    Ok(())
}

fn fetch_registered_public_key(
    public_key: &str,
    api_base: Option<String>,
) -> Result<ApiPublicKey> {
    let api_base = api_base.unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let public_key_path = public_key_to_path_segment(public_key);

    let url = format!(
        "{}/keys/{}",
        api_base.trim_end_matches('/'),
        public_key_path
    );

    let response = Client::new()
        .get(&url)
        .header(ACCEPT, "application/json")
        .send()
        .with_context(|| format!("failed to request public key from {}", url))?;

    let status = response.status();

    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        bail!("This signature doesn't match any registered public keys. Response body: {}", body);
    }

    let body: PublicKeyResponse = response
        .json()
        .context("failed to parse public key response")?;

    Ok(body.key)
}

fn normalize_base64(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect()
}

fn public_key_to_path_segment(public_key: &str) -> String {
    normalize_base64(public_key)
        .trim_end_matches('=')
        .replace('+', "-")
        .replace('/', "_")
}
