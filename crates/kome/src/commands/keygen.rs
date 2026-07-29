use std::{fs, path::Path, process::Command};

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{SigningKey, VerifyingKey};

use crate::cli::KeygenArgs;

pub fn run(args: KeygenArgs) -> Result<()> {
    let project_dir = std::env::current_dir().context("failed to resolve the project directory")?;
    run_in_project(args, &project_dir)
}

pub(crate) fn run_in_project(args: KeygenArgs, project_dir: &Path) -> Result<()> {
    match (args.private_key.exists(), args.public_key.exists()) {
        (true, true) => {
            verify_keypair(&args.private_key, &args.public_key)?;
            ensure_private_key_ignored(project_dir, &args.private_key)?;
            println!("Key pair already exists and matches.");
            println!("Private: {}", args.private_key.display());
            println!("Public:  {}", args.public_key.display());
            return Ok(());
        }
        (true, false) | (false, true) => {
            bail!("incomplete key pair exists; refusing to overwrite either key");
        }
        (false, false) => {}
    }

    let status = Command::new("msign")
        .arg("key")
        .arg("generate")
        .arg("--private-key")
        .arg(&args.private_key)
        .arg("--public-key")
        .arg(&args.public_key)
        .status()
        .context("failed to execute msign. is msign installed?")?;

    if !status.success() {
        bail!("msign key generate failed");
    }
    ensure_private_key_ignored(project_dir, &args.private_key)?;
    verify_keypair(&args.private_key, &args.public_key)?;

    println!("Private: {}", args.private_key.display());
    println!("Public:  {}", args.public_key.display());

    Ok(())
}

fn verify_keypair(private_key: &Path, public_key: &Path) -> Result<()> {
    let private_text = fs::read_to_string(private_key)
        .with_context(|| format!("failed to read {}", private_key.display()))?;
    let private_bytes: [u8; 32] = STANDARD
        .decode(private_text.trim())
        .context("application.key is not valid Base64")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("application.key must contain 32 raw Ed25519 bytes"))?;
    let signing_key = SigningKey::from_bytes(&private_bytes);

    let public_text = fs::read_to_string(public_key)
        .with_context(|| format!("failed to read {}", public_key.display()))?;
    let public_bytes: [u8; 32] = STANDARD
        .decode(public_text.trim())
        .context("application.pub is not valid Base64")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("application.pub must contain 32 raw Ed25519 bytes"))?;
    VerifyingKey::from_bytes(&public_bytes)
        .context("application.pub is not an Ed25519 public key")?;
    if signing_key.verifying_key().to_bytes() != public_bytes {
        bail!("application.key and application.pub do not form a matching Ed25519 key pair");
    }
    Ok(())
}

fn ensure_private_key_ignored(project_dir: &Path, private_key: &Path) -> Result<()> {
    let relative = if private_key.is_absolute() {
        let Ok(relative) = private_key.strip_prefix(project_dir) else {
            eprintln!(
                "warning: private key is outside the project; ensure it is ignored by VCS: {}",
                private_key.display()
            );
            return Ok(());
        };
        relative
    } else {
        private_key
    };
    ensure_private_key_ignored_in(project_dir, relative)
}

fn ensure_private_key_ignored_in(project_dir: &Path, private_key: &Path) -> Result<()> {
    let ignore_entry = gitignore_entry(private_key)?;
    let gitignore = project_dir.join(".gitignore");
    let mut text = match fs::read_to_string(&gitignore) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).context("failed to read .gitignore"),
    };
    if gitignore_covers(&text, &ignore_entry) {
        return Ok(());
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&ignore_entry);
    text.push('\n');
    fs::write(gitignore, text).context("failed to update .gitignore")
}

fn gitignore_covers(text: &str, entry: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        line == entry
            || (entry.starts_with("keys/") && entry.ends_with(".key") && line == "keys/*.key")
    })
}

fn gitignore_entry(private_key: &Path) -> Result<String> {
    let value = private_key
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("private key path is not UTF-8"))?
        .replace('\\', "/");
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gitignore_helper_keeps_existing_keys_glob() {
        assert!(gitignore_covers(
            "target/\ndist/\nkeys/*.key\n",
            "keys/application.key"
        ));
    }

    #[test]
    fn keygen_adds_private_key_to_gitignore_without_rewriting_existing_entries() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(temporary.path().join(".gitignore"), "target/").unwrap();

        ensure_private_key_ignored_in(temporary.path(), Path::new("secrets/developer.key"))
            .unwrap();
        let text = fs::read_to_string(temporary.path().join(".gitignore")).unwrap();
        assert_eq!(text, "target/\nsecrets/developer.key\n");

        ensure_private_key_ignored_in(temporary.path(), Path::new("secrets/developer.key"))
            .unwrap();
        let text = fs::read_to_string(temporary.path().join(".gitignore")).unwrap();
        assert_eq!(text, "target/\nsecrets/developer.key\n");
    }

    #[test]
    fn keypair_check_rejects_mismatched_public_key() {
        let temporary = tempfile::tempdir().unwrap();
        let private_path = temporary.path().join("application.key");
        let public_path = temporary.path().join("application.pub");
        let private = SigningKey::from_bytes(&[7; 32]);
        let other = SigningKey::from_bytes(&[9; 32]);
        fs::write(&private_path, STANDARD.encode(private.to_bytes())).unwrap();
        fs::write(
            &public_path,
            STANDARD.encode(other.verifying_key().to_bytes()),
        )
        .unwrap();
        assert!(verify_keypair(&private_path, &public_path).is_err());
    }

    #[test]
    fn absolute_project_key_is_added_to_project_gitignore() {
        let temporary = tempfile::tempdir().unwrap();
        let private_key = temporary.path().join("keys/application.key");
        fs::create_dir(private_key.parent().unwrap()).unwrap();

        ensure_private_key_ignored(temporary.path(), &private_key).unwrap();

        assert_eq!(
            fs::read_to_string(temporary.path().join(".gitignore")).unwrap(),
            "keys/application.key\n"
        );
    }

    #[test]
    fn key_outside_project_does_not_modify_project_gitignore() {
        let project = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        ensure_private_key_ignored(project.path(), &elsewhere.path().join("application.key"))
            .unwrap();
        assert!(!project.path().join(".gitignore").exists());
    }
}
