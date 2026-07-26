use std::{fs, path::Path};

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;

pub fn generate_keypair() -> (SigningKey, VerifyingKey) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    (signing_key, verifying_key)
}

pub fn write_private_key(path: &Path, key: &SigningKey) -> Result<()> {
    fs::write(path, STANDARD.encode(key.to_bytes()))
        .with_context(|| format!("failed to write private key: {}", path.display()))
}

pub fn write_public_key(path: &Path, key: &VerifyingKey) -> Result<()> {
    fs::write(path, STANDARD.encode(key.to_bytes()))
        .with_context(|| format!("failed to write public key: {}", path.display()))
}

pub fn read_private_key(path: &Path) -> Result<SigningKey> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read private key: {}", path.display()))?;

    let bytes = STANDARD
        .decode(text.trim())
        .context("private key is not valid base64")?;

    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("private key must be 32 bytes"))?;

    Ok(SigningKey::from_bytes(&bytes))
}

pub fn read_public_key(path: &Path) -> Result<VerifyingKey> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read public key: {}", path.display()))?;

    public_key_from_base64(text.trim())
}

pub fn public_key_to_base64(key: &VerifyingKey) -> String {
    STANDARD.encode(key.to_bytes())
}

pub fn public_key_from_base64(text: &str) -> Result<VerifyingKey> {
    let bytes = STANDARD
        .decode(text.trim())
        .context("public key is not valid base64")?;

    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("public key must be 32 bytes"))?;

    VerifyingKey::from_bytes(&bytes).context("invalid ed25519 public key")
}

pub fn sign(key: &SigningKey, message: &[u8]) -> String {
    let sig = key.sign(message);
    STANDARD.encode(sig.to_bytes())
}

pub fn verify(key: &VerifyingKey, message: &[u8], signature_b64: &str) -> Result<()> {
    let bytes = STANDARD
        .decode(signature_b64.trim())
        .context("signature is not valid base64")?;

    let sig = Signature::from_slice(&bytes).context("invalid ed25519 signature")?;

    if key.verify(message, &sig).is_err() {
        bail!("signature verification failed");
    }

    Ok(())
}
