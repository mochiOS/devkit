use std::{fs, path::Path, process::Command};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{cli::PackArgs, manifest::KomeManifest, project};

pub fn run(args: PackArgs) -> Result<()> {
    if args.legacy {
        return run_legacy(args);
    }

    let project_dir = args.project_dir;
    let manifest = project::read_manifest(&project_dir)?;
    let build_dir = if args.release {
        project_dir.join("target/release")
    } else {
        project_dir.join("target/debug")
    };
    let output = args.output.unwrap_or_else(|| {
        project_dir
            .join("dist")
            .join(format!("{}-unsigned.mpkg", manifest.package.name))
    });
    let staging_dir = project_dir.join("target/mpkg-staging");
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)
            .with_context(|| format!("failed to remove {}", staging_dir.display()))?;
    }
    fs::create_dir_all(staging_dir.join("payload/bundle"))
        .with_context(|| format!("failed to create {}", staging_dir.display()))?;

    let runtime_manifest =
        stage_application_payload(&project_dir, &build_dir, &staging_dir, &manifest)?;
    let manifest_path = staging_dir.join("manifest.toml");
    fs::write(
        &manifest_path,
        toml::to_string_pretty(&runtime_manifest).context("failed to serialize manifest.toml")?,
    )
    .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    let mut command = Command::new("mpack");
    command
        .arg("create")
        .arg("--manifest")
        .arg(&manifest_path)
        .arg("--payload")
        .arg(staging_dir.join("payload"))
        .arg("--output")
        .arg(&output);
    if args.force {
        command.arg("--force");
    }
    let status = command
        .status()
        .context("failed to execute mpack. is mpack installed?")?;
    if !status.success() {
        bail!("mpack create failed");
    }

    Ok(())
}

fn run_legacy(args: PackArgs) -> Result<()> {
    eprintln!("warning: legacy .pkg packaging does not support mochiOS AppStore");
    let mut command = Command::new("mpack");

    command.arg("pack");
    command.arg(&args.project_dir);

    if let Some(output) = args.output {
        command.arg("--output");
        command.arg(output);
    }

    if args.release {
        command.arg("--release");
    }

    if args.force {
        command.arg("--force");
    }

    let status = command
        .status()
        .context("failed to execute mpack. is mpack installed?")?;

    if !status.success() {
        bail!("mpack failed");
    }

    Ok(())
}

#[derive(Debug, Serialize)]
struct MpkgManifest {
    format: u32,
    package: MpkgPackage,
    file: Vec<MpkgFile>,
    binary: Vec<MpkgBinary>,
}

#[derive(Debug, Serialize)]
struct MpkgPackage {
    id: String,
    name: String,
    version: String,
    vendor: String,
    kind: String,
    architecture: String,
    abi: String,
}

#[derive(Debug, Serialize)]
struct MpkgFile {
    id: String,
    path: String,
    digest: String,
    size: u64,
    mode: String,
}

#[derive(Debug, Serialize)]
struct MpkgBinary {
    path: String,
    file: String,
    kind: String,
    requires: Vec<String>,
}

fn stage_application_payload(
    project_dir: &Path,
    build_dir: &Path,
    staging_dir: &Path,
    manifest: &KomeManifest,
) -> Result<MpkgManifest> {
    let entry_source = build_dir.join(&manifest.app.entry);
    if !entry_source.exists() {
        bail!(
            "entry file does not exist: {}. run `kome build` first",
            entry_source.display()
        );
    }

    let mut files = Vec::new();
    stage_bundle_file(
        &entry_source,
        &staging_dir.join("payload/bundle").join(&manifest.app.entry),
        "entry",
        &manifest.app.entry,
        Some(0o755),
        &mut files,
    )?;

    let icon_source = project_dir.join(&manifest.app.icon);
    if icon_source.exists() {
        stage_bundle_file(
            &icon_source,
            &staging_dir.join("payload/bundle").join(&manifest.app.icon),
            "icon",
            &manifest.app.icon,
            None,
            &mut files,
        )?;
    }

    for (index, resource) in manifest.resources.files.iter().enumerate() {
        let source = project_dir.join(resource);
        if !source.exists() {
            bail!("resource does not exist: {}", source.display());
        }
        stage_bundle_file(
            &source,
            &staging_dir.join("payload/bundle").join(resource),
            &format!("resource-{index}"),
            resource,
            None,
            &mut files,
        )?;
    }

    Ok(MpkgManifest {
        format: 1,
        package: MpkgPackage {
            id: manifest.package.id.clone(),
            name: manifest.package.name.clone(),
            version: manifest.package.version.clone(),
            vendor: manifest.package.developer.clone(),
            kind: "application".to_string(),
            architecture: "x86_64".to_string(),
            abi: "mochios-1".to_string(),
        },
        file: files,
        binary: vec![MpkgBinary {
            path: format!(
                "/applications/{}.app/{}",
                manifest.package.name, manifest.app.entry
            ),
            file: "entry".to_string(),
            kind: "application".to_string(),
            requires: manifest.capabilities.required.clone(),
        }],
    })
}

fn stage_bundle_file(
    source: &Path,
    dest: &Path,
    id: &str,
    bundle_path: &str,
    mode: Option<u32>,
    files: &mut Vec<MpkgFile>,
) -> Result<()> {
    validate_bundle_path(bundle_path)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::copy(source, dest)
        .with_context(|| format!("failed to copy {} to {}", source.display(), dest.display()))?;
    let data = fs::read(dest).with_context(|| format!("failed to read {}", dest.display()))?;
    files.push(MpkgFile {
        id: id.to_string(),
        path: format!("$/{bundle_path}"),
        digest: format!("sha256:{}", hex(&Sha256::digest(&data))),
        size: data.len() as u64,
        mode: format!("{:04o}", mode.unwrap_or(file_mode(source)?)),
    });
    Ok(())
}

fn validate_bundle_path(path: &str) -> Result<()> {
    if path.trim().is_empty()
        || path.starts_with('/')
        || path.contains("..")
        || path.contains('\\')
        || path.as_bytes().contains(&0)
    {
        bail!("bundle path must be relative inside project: {path}");
    }
    Ok(())
}

#[cfg(unix)]
fn file_mode(path: &Path) -> Result<u32> {
    use std::os::unix::fs::PermissionsExt;
    let metadata =
        fs::metadata(path).with_context(|| format!("failed to stat {}", path.display()))?;
    Ok(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn file_mode(_path: &Path) -> Result<u32> {
    Ok(0o644)
}

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    output
}
