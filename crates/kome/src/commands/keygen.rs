use std::{fs, path::Path, process::Command};

use anyhow::{bail, Context, Result};

use crate::cli::KeygenArgs;

pub fn run(args: KeygenArgs) -> Result<()> {
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
        bail!("msign keygen failed");
    }
    ensure_private_key_ignored(&args.private_key)?;

    Ok(())
}

fn ensure_private_key_ignored(private_key: &Path) -> Result<()> {
    if private_key.is_absolute() {
        eprintln!(
            "warning: private key path is absolute; ensure it is ignored by VCS: {}",
            private_key.display()
        );
        return Ok(());
    }

    ensure_private_key_ignored_in(Path::new("."), private_key)
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
}
