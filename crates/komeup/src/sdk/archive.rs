//! Manifest-bound extraction into private staging. SDK programs are never executed here.
use super::{relative, valid_hash, Manifest, INSTALLED_MANIFEST, MAX_EXPANDED, RECEIPT};
use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// Extract only declared files under sdk/, hashing bytes before committing any live installation.
pub(super) fn extract(input: &Path, root: &Path, manifest: &Manifest) -> Result<()> {
    if manifest.files.is_empty() || manifest.files.len() > 50_000 {
        bail!("Invalid SDK file inventory size");
    }
    for (name, record) in &manifest.files {
        relative(name)?;
        if name == RECEIPT || name == INSTALLED_MANIFEST {
            bail!("SDK archive contains a reserved manager file");
        }
        match (&record.sha256, record.size, &record.symlink) {
            (Some(hash), Some(size), None) if valid_hash(hash) && size <= MAX_EXPANDED => (),
            (None, None, Some(link)) => {
                link_target(Path::new(name), link)?;
            }
            _ => bail!("Invalid SDK inventory entry: {name}"),
        }
    }
    let decoder = GzDecoder::new(File::open(input)?).take(MAX_EXPANDED + 1);
    let mut archive = tar::Archive::new(decoder);
    let mut seen = BTreeSet::new();
    let mut payload = BTreeSet::new();
    let mut links = Vec::new();
    let mut total = 0u64;
    for (index, entry) in archive.entries()?.enumerate() {
        if index >= 100_000 {
            bail!("SDK archive has too many entries");
        }
        let mut entry = entry?;
        let name = entry.path()?.into_owned();
        let components: Vec<_> = name.components().collect();
        if components.first() != Some(&Component::Normal("sdk".as_ref())) {
            bail!("SDK archive must use a single sdk/ root");
        }
        let relative_name: PathBuf = components.iter().skip(1).collect();
        let kind = entry.header().entry_type();
        if relative_name.as_os_str().is_empty() {
            if !kind.is_dir() || !seen.insert(PathBuf::new()) {
                bail!("Invalid SDK root entry");
            }
            continue;
        }
        let text = relative_name.to_str().context("Non-UTF-8 SDK path")?;
        relative(text)?;
        if !seen.insert(relative_name.clone()) {
            bail!("Duplicate SDK archive entry: {text}");
        }
        let output = root.join(&relative_name);
        let mode = entry.header().mode()?;
        if mode & 0o7000 != 0 {
            bail!("Privileged SDK archive permissions are forbidden");
        }
        if kind.is_dir() {
            ensure_parents(root, &relative_name)?;
            continue;
        }
        let record = manifest
            .files
            .get(text)
            .with_context(|| format!("Undeclared SDK file: {text}"))?;
        payload.insert(text.to_string());
        if kind.is_symlink() {
            let link = entry.link_name()?.context("SDK symlink has no target")?;
            let link = link.to_str().context("Non-UTF-8 SDK link")?;
            if record.symlink.as_deref() != Some(link) {
                bail!("SDK symlink differs from the manifest: {text}");
            }
            link_target(&relative_name, link)?;
            links.push((relative_name, link.to_string()));
        } else if kind.is_file() {
            if record.symlink.is_some() || record.size != Some(entry.size()) {
                bail!("SDK file type/size differs from the manifest: {text}");
            }
            total = total
                .checked_add(entry.size())
                .context("SDK size overflow")?;
            if total > MAX_EXPANDED {
                bail!("Expanded SDK exceeds 8 GiB");
            }
            ensure_parents(root, relative_name.parent().unwrap_or(Path::new("")))?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)?;
            let mut digest = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let len = entry.read(&mut buffer)?;
                if len == 0 {
                    break;
                }
                digest.update(&buffer[..len]);
                file.write_all(&buffer[..len])?;
            }
            if record.sha256.as_deref() != Some(&format!("{:x}", digest.finalize())) {
                bail!("SDK file hash mismatch: {text}");
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(mode & 0o777))?;
            }
            file.sync_all()?;
        } else {
            bail!("Unsupported SDK archive entry type: {text}");
        }
    }
    if payload != manifest.files.keys().cloned().collect() {
        bail!("SDK archive is incomplete");
    }
    // Files/directories first, symlinks last: archive order cannot redirect writes outside staging.
    for (name, target) in &links {
        ensure_parents(root, name.parent().unwrap_or(Path::new("")))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, root.join(name))?;
        #[cfg(not(unix))]
        bail!("SDK symlink installation requires a Unix-like host");
    }
    for (name, _) in &links {
        if !root
            .join(name)
            .canonicalize()?
            .starts_with(root.canonicalize()?)
        {
            bail!("SDK link escapes its root");
        }
    }
    Ok(())
}

fn ensure_parents(root: &Path, relative: &Path) -> Result<()> {
    let mut path = root.to_path_buf();
    for part in relative.components() {
        let Component::Normal(part) = part else {
            bail!("Invalid SDK directory path");
        };
        path.push(part);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => (),
            Ok(_) => bail!("SDK parent is not a regular directory"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir(&path)?,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn link_target(name: &Path, target: &str) -> Result<PathBuf> {
    let target = Path::new(target);
    if target.is_absolute() || target.as_os_str().as_encoded_bytes().contains(&b'\\') {
        bail!("Invalid SDK symlink target");
    }
    let mut result = name.parent().unwrap_or(Path::new("")).to_path_buf();
    for part in target.components() {
        match part {
            Component::Normal(value) => result.push(value),
            Component::CurDir => (),
            Component::ParentDir if result.pop() => (),
            _ => bail!("SDK symlink escapes its root"),
        }
    }
    Ok(result)
}

/// Recheck every installed payload hash without running the compiler or other downloaded programs.
pub(super) fn verify_installed(root: &Path, manifest: &Manifest) -> Result<()> {
    for (name, record) in &manifest.files {
        let path = root.join(relative(name)?);
        if let Some(expected) = &record.symlink {
            if fs::read_link(&path)? != Path::new(expected)
                || !path.canonicalize()?.starts_with(root.canonicalize()?)
            {
                bail!("SDK link changed: {name}");
            }
        } else {
            if fs::symlink_metadata(&path)?.file_type().is_symlink() {
                bail!("SDK file became a symlink: {name}");
            }
            let mut file = File::open(&path)?;
            if Some(file.metadata()?.len()) != record.size {
                bail!("SDK file size changed: {name}");
            }
            let mut digest = Sha256::new();
            let mut buffer = [0u8; 64 * 1024];
            loop {
                let count = file.read(&mut buffer)?;
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
            }
            if record.sha256.as_deref() != Some(&format!("{:x}", digest.finalize())) {
                bail!("SDK file hash changed: {name}");
            }
        }
    }
    Ok(())
}
