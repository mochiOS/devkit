//! Build-time bindings for code in an owned staging tree. No SDK generation,
//! ELF layout, trust decision or execution authorization is performed here.
use crate::cli::BindBuiltInArgs;
use anyhow::{ensure, Context, Result};
use mochios_file_publication::PrivateDirectory;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::Path,
};

fn absolute(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains(['\\', '\0'])
        && path[1..]
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}

fn mapped(path: &str, maps: &[(String, String)]) -> String {
    for (from, to) in maps {
        if let Some(tail) = path.strip_prefix(from) {
            if tail.is_empty() || tail.starts_with('/') {
                return format!("{to}{tail}");
            }
        }
    }
    path.into()
}

#[cfg(unix)]
fn mode(metadata: &fs::Metadata) -> Result<u32> {
    use std::os::unix::fs::PermissionsExt;
    Ok(metadata.permissions().mode() & 0o7777)
}
#[cfg(not(unix))]
fn mode(_: &fs::Metadata) -> Result<u32> {
    anyhow::bail!("binding code requires Unix file modes")
}

#[cfg(unix)]
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}
#[cfg(not(unix))]
fn same_file(_: &fs::Metadata, _: &fs::Metadata) -> bool {
    false
}

fn measure(root: &Path, path: &str, output: &Path) -> Result<(String, u64, u32)> {
    let path = root
        .join(&path[1..])
        .canonicalize()
        .with_context(|| format!("missing staged code: {path}"))?;
    ensure!(
        path.starts_with(root),
        "staged code escapes rootfs: {}",
        path.display()
    );
    let mut file = fs::File::open(&path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() != 0,
        "staged code is not a nonempty regular file: {}",
        path.display()
    );
    if let Ok(out) = fs::metadata(output) {
        ensure!(
            !same_file(&metadata, &out),
            "manifest output aliases staged code"
        );
    }
    let mut remaining = metadata.len();
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    while remaining != 0 {
        let capacity = remaining.min(buffer.len() as u64) as usize;
        let n = file.read(&mut buffer[..capacity])?;
        ensure!(n != 0, "staged code was truncated during measurement");
        digest.update(&buffer[..n]);
        remaining -= n as u64;
    }
    let mut extra = [0];
    let at_eof = file.read(&mut extra)? == 0;
    let after = file.metadata()?;
    ensure!(
        at_eof
            && after.len() == metadata.len()
            && after.modified()? == metadata.modified()?
            && mode(&after)? == mode(&metadata)?,
        "staged code changed during measurement"
    );
    Ok((
        format!("sha256:{:x}", digest.finalize()),
        metadata.len(),
        mode(&metadata)?,
    ))
}

pub(crate) fn bind(args: BindBuiltInArgs) -> Result<()> {
    let (bytes, manifest) = super::builtin::read_manifest(&args.manifest)?;
    let mut document: toml::Table = std::str::from_utf8(&bytes)?.parse()?;
    let root = args
        .rootfs
        .canonicalize()
        .context("invalid rootfs directory")?;
    ensure!(root.is_dir(), "rootfs is not a directory");
    let mut maps = Vec::new();
    let mut prefixes = BTreeSet::new();
    for mapping in args.map_path {
        let (from, to) = mapping
            .split_once('=')
            .context("map-path requires FROM=TO")?;
        ensure!(
            absolute(from) && absolute(to) && prefixes.insert(from.to_owned()),
            "invalid or duplicate map-path"
        );
        maps.push((from.to_owned(), to.to_owned()));
    }
    maps.sort_by_key(|(from, _)| std::cmp::Reverse(from.len()));
    let excluded: BTreeSet<_> = args.exclude_binary.iter().cloned().collect();
    ensure!(
        excluded.len() == args.exclude_binary.len(),
        "duplicate binary exclusion"
    );
    for path in &excluded {
        ensure!(
            manifest.binary(path).is_some(),
            "excluded binary is not declared: {path}"
        );
    }
    let mut binaries = document
        .remove("binary")
        .unwrap_or(toml::Value::Array(Vec::new()))
        .as_array()
        .context("binary must be an array")?
        .clone();
    ensure!(
        binaries.len() == manifest.binaries.len(),
        "bind-built-in requires explicit binary entries"
    );
    let mut files = document
        .remove("file")
        .unwrap_or(toml::Value::Array(Vec::new()))
        .as_array()
        .context("file must be an array")?
        .clone();
    let mut selected = Vec::new();
    for (index, (mut entry, binary)) in binaries.drain(..).zip(&manifest.binaries).enumerate() {
        if excluded.contains(&binary.path) {
            continue;
        }
        ensure!(
            absolute(&binary.path),
            "binary path must be canonical and absolute: {}",
            binary.path
        );
        let (digest, size, mode) = measure(&root, &mapped(&binary.path, &maps), &args.output)?;
        if binary.kind.as_deref() != Some("library") {
            ensure!(
                mode & 0o111 != 0,
                "staged code has no execute permission: {}",
                binary.path
            );
        }
        if let Some(id) = &binary.file {
            let old = manifest
                .files
                .iter()
                .find(|f| &f.id == id)
                .context("binary references an absent file entry")?;
            ensure!(
                old.path == binary.path
                    && old.digest == digest
                    && old.size == size
                    && old.mode == mode,
                "existing code binding does not match staged bytes: {}",
                binary.path
            );
        } else {
            let id = format!("mochios-code-{index}");
            ensure!(
                !manifest.files.iter().any(|f| f.id == id),
                "generated code file ID already exists: {id}"
            );
            entry
                .as_table_mut()
                .context("invalid binary entry")?
                .insert("file".into(), id.clone().into());
            let mut file = toml::Table::new();
            file.insert("id".into(), id.into());
            file.insert("path".into(), binary.path.clone().into());
            file.insert("digest".into(), digest.into());
            file.insert(
                "size".into(),
                i64::try_from(size)
                    .context("staged code is too large")?
                    .into(),
            );
            file.insert("mode".into(), format!("{mode:04o}").into());
            files.push(file.into());
        }
        selected.push(entry);
    }
    document.insert("binary".into(), selected.into());
    document.insert("file".into(), files.into());
    let text = toml::to_string(&document)?;
    mochios_package_manifest::parse_manifest_checked(&text)
        .map_err(|e| anyhow::anyhow!("invalid bound manifest: {e:?}"))?;
    if let Ok(metadata) = fs::symlink_metadata(&args.output) {
        ensure!(metadata.is_file(), "manifest output is not a regular file");
    }
    let parent = args
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let directory = PrivateDirectory::new_in(parent, "msign-bind")?;
    let mut file = directory.create_file("manifest.toml")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    file.write_all(text.as_bytes())?;
    directory.publish("manifest.toml", &file, &args.output)?;
    Ok(())
}
