use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, COOKIE};
use serde::Deserialize;

use crate::{
    cli::VerifyArgs,
    crypto,
    package,
    signature::{PackageSignature, SIGNATURE_ALGORITHM, SIGNATURE_VERSION},
};

const DEFAULT_API_BASE: &str = "https://api.mochios.org/v1";

#[derive(Debug, Deserialize)]
struct KeysResponse {
    keys: Vec<ApiPublicKey>,
}

#[derive(Debug, Deserialize)]
struct ApiPublicKey {
    key_id: String,
    public_key: String,
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

    let api_public_key = if args.local {
        None
    } else {
        Some(fetch_registered_public_key(
            &signature.key_id,
            args.api_base,
            args.session,
        )?)
    };

    if let Some(api_public_key) = &api_public_key {
        if api_public_key != &signature.public_key {
            bail!("public key mismatch: package key does not match API registered key");
        }
    }

    let public_key = match &args.pubkey {
        Some(path) => crypto::read_public_key(path)
            .with_context(|| format!("failed to read public key from {}", path.display()))?,
        None => {
            let public_key_base64 = api_public_key.as_ref().unwrap_or(&signature.public_key);
            crypto::public_key_from_base64(public_key_base64)?
        }
    };

    let message = PackageSignature::signing_message(
        &signature.package_hash,
        &signature.key_id,
        &signature.public_key,
    );

    crypto::verify(&public_key, &message, &signature.signature)?;

    println!("verified: {}", args.package.display());
    println!("key_id: {}", signature.key_id);

    if args.local {
        println!("mode: local");
    } else {
        println!("mode: api");
    }

    Ok(())
}

fn fetch_registered_public_key(
    key_id: &str,
    api_base: Option<String>,
    session: Option<String>,
) -> Result<String> {
    let api_base = api_base.unwrap_or_else(|| DEFAULT_API_BASE.to_string());
    let url = format!("{}/keys", api_base.trim_end_matches('/'));

    let session = session
        .or_else(|| std::env::var("MOCHIOS_APPSTORE_SESSION").ok())
        .context("API session is required. Pass --session or set MOCHIOS_APPSTORE_SESSION")?;

    let cookie = format!("PHPSESSID={}", session);

    let response = Client::new()
        .get(&url)
        .header(ACCEPT, "application/json")
        .header(COOKIE, cookie)
        .send()
        .with_context(|| format!("failed to request public keys from {}", url))?;

    let status = response.status();

    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        bail!("failed to fetch public keys: HTTP {}: {}", status, body);
    }

    let body: KeysResponse = response
        .json()
        .context("failed to parse public keys response")?;

    let key = body
        .keys
        .into_iter()
        .find(|key| key.key_id == key_id)
        .with_context(|| format!("public key is not registered: {}", key_id))?;

    if key.revoked_at.is_some() {
        bail!("public key has been revoked: {}", key_id);
    }

    Ok(key.public_key)
}