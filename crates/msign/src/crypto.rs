use std::{fs, path::Path};

use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand_core::OsRng;

pub fn generate_keypair() -> (SigningKey, VerifyingKey) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    (signing_key, verifying_key)
}

pub fn write_private_key(path: &Path, key: &SigningKey) -> Result<()> {
    write_new_file(path, STANDARD.encode(key.to_bytes()).as_bytes(), 0o600)
        .with_context(|| format!("failed to write private key: {}", path.display()))
}

pub fn write_public_key(path: &Path, key: &VerifyingKey) -> Result<()> {
    write_new_file(path, STANDARD.encode(key.to_bytes()).as_bytes(), 0o644)
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

fn write_new_file(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    open_with_mode(&mut options, mode);
    let mut file = options.open(path)?;
    use std::io::Write;
    file.write_all(bytes)?;
    Ok(())
}

#[cfg(unix)]
fn open_with_mode(options: &mut fs::OpenOptions, mode: u32) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(mode);
}

#[cfg(not(unix))]
fn open_with_mode(_options: &mut fs::OpenOptions, _mode: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_keypair_round_trips_and_refuses_overwrite() {
        let temporary = tempfile::tempdir().unwrap();
        let private_path = temporary.path().join("application.key");
        let public_path = temporary.path().join("application.pub");
        let (private_key, public_key) = generate_keypair();
        write_private_key(&private_path, &private_key).unwrap();
        write_public_key(&public_path, &public_key).unwrap();

        assert_eq!(
            read_private_key(&private_path).unwrap().to_bytes(),
            private_key.to_bytes()
        );
        assert_eq!(
            read_public_key(&public_path).unwrap().to_bytes(),
            public_key.to_bytes()
        );
        assert!(write_private_key(&private_path, &private_key).is_err());
        assert!(write_public_key(&public_path, &public_key).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn private_key_is_written_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let private_path = temporary.path().join("application.key");
        let (private_key, _) = generate_keypair();

        write_private_key(&private_path, &private_key).unwrap();

        let mode = fs::metadata(&private_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
