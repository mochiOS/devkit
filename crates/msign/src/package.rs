use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use sha2::{Digest, Sha256};
use tar::{Archive, Builder, Header};
use tempfile::NamedTempFile;

use crate::signature::{PackageSignature, SIGNATURE_PATH};

#[derive(Debug, Clone)]
struct PackageEntry {
    path: PathBuf,
    data: Vec<u8>,
    mode: u32,
}

pub fn calculate_package_hash(package: &Path) -> Result<String> {
    let entries = read_entries_without_signature(package)?;

    let mut hashes = Vec::new();

    for entry in entries {
        let mut file_hash = Sha256::new();
        file_hash.update(&entry.data);

        hashes.push((
            normalize_path(&entry.path)?,
            entry.data.len(),
            STANDARD.encode(file_hash.finalize()),
        ));
    }

    hashes.sort_by(|a, b| a.0.cmp(&b.0));

    let mut root = Sha256::new();

    for (path, size, hash) in hashes {
        root.update(path.as_bytes());
        root.update(b"\0");
        root.update(size.to_string().as_bytes());
        root.update(b"\0");
        root.update(hash.as_bytes());
        root.update(b"\n");
    }

    Ok(STANDARD.encode(root.finalize()))
}

pub fn read_signature(package: &Path) -> Result<PackageSignature> {
    let file = File::open(package)
        .with_context(|| format!("failed to open package: {}", package.display()))?;

    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    for entry in archive.entries().context("failed to read package entries")? {
        let mut entry = entry.context("failed to read package entry")?;
        let path = entry.path().context("failed to read package entry path")?;

        if normalize_path(&path)? == SIGNATURE_PATH {
            let mut text = String::new();
            entry
                .read_to_string(&mut text)
                .context("failed to read signature.toml")?;

            return toml::from_str(&text).context("failed to parse signature.toml");
        }
    }

    bail!("package does not contain {}", SIGNATURE_PATH);
}

pub fn write_signature(package: &Path, output: &Path, signature: &PackageSignature) -> Result<()> {
    let mut entries = read_entries_without_signature(package)?;

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    let temp = NamedTempFile::new().context("failed to create temporary package")?;
    let temp_path = temp.path().to_path_buf();

    {
        let file = File::create(&temp_path).context("failed to create output package")?;
        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = Builder::new(encoder);

        for entry in entries {
            append_file(&mut builder, &entry.path, &entry.data, entry.mode)?;
        }

        let signature_text =
            toml::to_string_pretty(signature).context("failed to serialize signature.toml")?;

        append_file(
            &mut builder,
            Path::new(SIGNATURE_PATH),
            signature_text.as_bytes(),
            0o644,
        )?;

        builder.finish().context("failed to finish tar archive")?;
    }

    std::fs::copy(&temp_path, output)
        .with_context(|| format!("failed to write output package: {}", output.display()))?;

    Ok(())
}

fn read_entries_without_signature(package: &Path) -> Result<Vec<PackageEntry>> {
    let file = File::open(package)
        .with_context(|| format!("failed to open package: {}", package.display()))?;

    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);

    let mut entries = Vec::new();

    for entry in archive.entries().context("failed to read package entries")? {
        let mut entry = entry.context("failed to read package entry")?;

        if !entry.header().entry_type().is_file() {
            continue;
        }

        let path = entry
            .path()
            .context("failed to read package entry path")?
            .to_path_buf();

        if normalize_path(&path)? == SIGNATURE_PATH {
            continue;
        }

        let mode = entry.header().mode().unwrap_or(0o644);

        let mut data = Vec::new();
        entry
            .read_to_end(&mut data)
            .context("failed to read package entry data")?;

        entries.push(PackageEntry { path, data, mode });
    }

    Ok(entries)
}

fn append_file<W: Write>(
    builder: &mut Builder<W>,
    path: &Path,
    data: &[u8],
    mode: u32,
) -> Result<()> {
    let mut header = Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(mode);
    header.set_cksum();

    builder
        .append_data(&mut header, path, data)
        .with_context(|| format!("failed to append {}", path.display()))
}

fn normalize_path(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("package path is not valid UTF-8"))?
        .replace('\\', "/");

    let text = text.trim_start_matches("./");

    if text.starts_with('/') || text.contains("../") || text == ".." {
        bail!("invalid package path: {}", text);
    }

    Ok(text.to_string())
}