use std::{
    collections::BTreeSet,
    fs,
    io::{Cursor, Write},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use tar::{Builder, EntryType, Header};
use tempfile::NamedTempFile;

use crate::cli::CreateArgs;

const MPKG_MAGIC: &[u8; 4] = b"MPKG";
const MPKG_HEADER_LEN: usize = 32;

#[derive(Debug, Clone)]
struct MpkgEntry {
    path: String,
    source: PathBuf,
    mode: u32,
}

pub fn create(args: CreateArgs) -> Result<()> {
    if args.output.exists() {
        if !args.force {
            bail!(
                "output already exists: {}. use --force to overwrite",
                args.output.display()
            );
        }
        fs::remove_file(&args.output)
            .with_context(|| format!("failed to replace {}", args.output.display()))?;
    }

    let manifest = fs::read(&args.manifest)
        .with_context(|| format!("failed to read {}", args.manifest.display()))?;
    let payload_entries = collect_payload_entries(&args.payload)?;
    write_mpkg(&args.output, &manifest, &payload_entries)?;

    println!("created: {}", args.output.display());
    println!("format: MPKG v1");
    println!("payload_entries: {}", payload_entries.len());

    Ok(())
}

fn collect_payload_entries(payload_dir: &Path) -> Result<Vec<MpkgEntry>> {
    if !payload_dir.is_dir() {
        bail!("payload is not a directory: {}", payload_dir.display());
    }

    let mut entries = Vec::new();
    collect_payload_entries_recursive(payload_dir, payload_dir, &mut entries)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));

    let mut seen = BTreeSet::new();
    for entry in &entries {
        if !seen.insert(entry.path.clone()) {
            bail!("duplicate MPKG path: {}", entry.path);
        }
    }

    Ok(entries)
}

fn collect_payload_entries_recursive(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<MpkgEntry>,
) -> Result<()> {
    let mut children = fs::read_dir(directory)
        .with_context(|| format!("failed to read {}", directory.display()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| format!("failed to read {}", directory.display()))?;
    children.sort_by_key(|entry| entry.path());

    for child in children {
        let source = child.path();
        let metadata = fs::symlink_metadata(&source)
            .with_context(|| format!("failed to stat {}", source.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("MPKG payload cannot contain symlink: {}", source.display());
        }
        if metadata.is_dir() {
            collect_payload_entries_recursive(root, &source, entries)?;
            continue;
        }
        if !metadata.is_file() {
            bail!("MPKG payload must be regular files: {}", source.display());
        }
        let relative = source
            .strip_prefix(root)
            .with_context(|| format!("failed to relativize {}", source.display()))?;
        let path = format!("payload/{}", normalize_relative_path(relative)?);
        entries.push(MpkgEntry {
            path,
            source,
            mode: 0o644,
        });
    }

    Ok(())
}

fn write_mpkg(output: &Path, manifest: &[u8], payload_entries: &[MpkgEntry]) -> Result<()> {
    let mut tar_bytes = Vec::new();
    {
        let mut builder = Builder::new(&mut tar_bytes);
        append_bytes(&mut builder, "manifest.toml", manifest, 0o644)?;
        for entry in payload_entries {
            let data = fs::read(&entry.source)
                .with_context(|| format!("failed to read {}", entry.source.display()))?;
            append_bytes(&mut builder, &entry.path, &data, entry.mode)?;
        }
        builder
            .finish()
            .context("failed to finish MPKG tar stream")?;
    }

    let expanded_size = u64::try_from(tar_bytes.len()).context("MPKG is too large")?;
    let mut header = [0u8; MPKG_HEADER_LEN];
    header[..4].copy_from_slice(MPKG_MAGIC);
    header[4..6].copy_from_slice(&1u16.to_le_bytes());
    header[6..8].copy_from_slice(&0u16.to_le_bytes());
    header[8..10].copy_from_slice(&(MPKG_HEADER_LEN as u16).to_le_bytes());
    header[10] = 0;
    header[11] = 0;
    header[12..20].copy_from_slice(&expanded_size.to_le_bytes());

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut temporary = NamedTempFile::new_in(output.parent().unwrap_or_else(|| Path::new(".")))
        .context("failed to create temporary MPKG")?;
    temporary
        .write_all(&header)
        .and_then(|_| temporary.write_all(&tar_bytes))
        .context("failed to write temporary MPKG")?;
    temporary
        .flush()
        .context("failed to flush temporary MPKG")?;
    temporary
        .persist(output)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", output.display()))?;

    Ok(())
}

fn append_bytes<W: Write>(
    builder: &mut Builder<W>,
    path: &str,
    data: &[u8],
    mode: u32,
) -> Result<()> {
    validate_mpkg_path(path)?;
    let mut header = Header::new_ustar();
    header.set_size(data.len() as u64);
    header.set_mode(mode & 0o777);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_entry_type(EntryType::Regular);
    header.set_cksum();
    builder
        .append_data(&mut header, path, Cursor::new(data))
        .with_context(|| format!("failed to append {path}"))
}

fn normalize_relative_path(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("MPKG path is not UTF-8"))?
        .replace('\\', "/");
    validate_relative_path(&value)?;
    Ok(value)
}

fn validate_mpkg_path(path: &str) -> Result<()> {
    validate_relative_path(path)?;
    if path != "manifest.toml"
        && !path.starts_with("payload/root/")
        && !path.starts_with("payload/bundle/")
    {
        bail!("MPKG contains entry outside allowed roots: {path}");
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.ends_with('/')
        || path.contains("//")
        || path.contains('\\')
        || path.as_bytes().contains(&0)
        || path
            .split('/')
            .any(|segment| segment == "." || segment == ".." || segment.is_empty())
    {
        bail!("invalid MPKG path: {path}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_stable_mpkg_header() {
        let temporary = tempfile::tempdir().unwrap();
        let payload = temporary.path().join("payload");
        fs::create_dir_all(payload.join("bundle")).unwrap();
        fs::write(payload.join("bundle/entry.elf"), b"elf").unwrap();
        let output = temporary.path().join("app.mpkg");
        write_mpkg(
            &output,
            b"format = 1\n",
            &collect_payload_entries(&payload).unwrap(),
        )
        .unwrap();
        let bytes = fs::read(output).unwrap();
        assert_eq!(&bytes[..4], b"MPKG");
        assert_eq!(u16::from_le_bytes([bytes[4], bytes[5]]), 1);
        assert_eq!(u16::from_le_bytes([bytes[6], bytes[7]]), 0);
        assert_eq!(u16::from_le_bytes([bytes[8], bytes[9]]), 32);
        assert_eq!(bytes[10], 0);
        assert_eq!(bytes[11], 0);
        assert!(bytes[20..32].iter().all(|byte| *byte == 0));
        assert_eq!(
            u64::from_le_bytes(bytes[12..20].try_into().unwrap()) as usize,
            bytes.len() - MPKG_HEADER_LEN
        );
    }

    #[test]
    fn generation_is_deterministic_for_same_input() {
        let temporary = tempfile::tempdir().unwrap();
        let payload = temporary.path().join("payload");
        fs::create_dir_all(payload.join("bundle")).unwrap();
        fs::write(payload.join("bundle/entry.elf"), b"elf").unwrap();
        fs::write(payload.join("bundle/resource.txt"), b"resource").unwrap();
        let entries = collect_payload_entries(&payload).unwrap();
        let first = temporary.path().join("first.mpkg");
        let second = temporary.path().join("second.mpkg");

        write_mpkg(&first, b"format = 1\n", &entries).unwrap();
        write_mpkg(&second, b"format = 1\n", &entries).unwrap();

        assert_eq!(fs::read(first).unwrap(), fs::read(second).unwrap());
    }

    #[test]
    fn rejects_forbidden_payload_paths() {
        assert!(validate_mpkg_path("payload/../manifest.toml").is_err());
        assert!(validate_mpkg_path("payload\\bundle\\entry.elf").is_err());
        assert!(validate_mpkg_path("signatures/manifest.sig").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_payload() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().unwrap();
        let payload = temporary.path().join("payload");
        fs::create_dir_all(payload.join("bundle")).unwrap();
        fs::write(temporary.path().join("target"), b"target").unwrap();
        symlink(temporary.path().join("target"), payload.join("bundle/link")).unwrap();

        assert!(collect_payload_entries(&payload).is_err());
    }
}
