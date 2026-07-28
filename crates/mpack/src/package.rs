use std::{
    fs::{self, File},
    io::{Cursor, Write},
    path::Path,
};

use anyhow::{bail, Context, Result};
use flate2::{write::GzEncoder, Compression};
use tar::{Builder, Header};
use tempfile::NamedTempFile;

use crate::{
    app_files,
    cli::PackArgs,
    manifest::{self, AboutToml, RuntimeManifestToml},
};

pub fn pack(args: PackArgs) -> Result<()> {
    let project_dir = args.project_dir;
    let manifest = manifest::read_kome_manifest(&project_dir)?;

    let build_dir = if args.release {
        project_dir.join("target/release")
    } else {
        project_dir.join("target/debug")
    };

    let output = args.output.unwrap_or_else(|| {
        project_dir
            .join("target/package")
            .join(format!("{}.pkg", manifest.package.name))
    });

    if output.exists() && !args.force {
        bail!(
            "output already exists: {}. use --force to overwrite",
            output.display()
        );
    }

    if output.exists() {
        fs::remove_file(&output)
            .with_context(|| format!("failed to remove {}", output.display()))?;
    }

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    let files = app_files::collect_package_files(&project_dir, &build_dir, &manifest)?;

    let about = manifest::make_about_toml(&manifest);
    let runtime_manifest = manifest::make_runtime_manifest(&manifest);

    write_package(&output, &files, &about, &runtime_manifest)?;

    println!("packed: {}", output.display());
    println!("name: {}", manifest.package.name);
    println!("id: {}", manifest.package.id);
    println!("version: {}", manifest.package.version);

    Ok(())
}

fn write_package(
    output: &Path,
    files: &[app_files::PackageFile],
    about: &AboutToml,
    runtime_manifest: &RuntimeManifestToml,
) -> Result<()> {
    let temp = NamedTempFile::new().context("failed to create temporary package")?;

    {
        let file = File::create(temp.path()).context("failed to create temporary package file")?;

        let encoder = GzEncoder::new(file, Compression::default());
        let mut builder = Builder::new(encoder);

        append_bytes(
            &mut builder,
            Path::new("about.toml"),
            toml::to_string_pretty(about)?.as_bytes(),
            0o644,
        )?;

        append_bytes(
            &mut builder,
            Path::new("manifest.toml"),
            toml::to_string_pretty(runtime_manifest)?.as_bytes(),
            0o644,
        )?;

        let mut sorted_files = files.to_vec();
        sorted_files.sort_by(|a, b| a.dest.cmp(&b.dest));

        for file in sorted_files {
            append_file(&mut builder, &file.source, &file.dest)?;
        }

        builder.finish().context("failed to finish package")?;
    }

    fs::copy(temp.path(), output)
        .with_context(|| format!("failed to write {}", output.display()))?;

    Ok(())
}

fn append_file<W: Write>(builder: &mut Builder<W>, src: &Path, dest: &Path) -> Result<()> {
    let data = fs::read(src).with_context(|| format!("failed to read {}", src.display()))?;

    append_bytes(builder, dest, &data, file_mode(src)?)
}

fn append_bytes<W: Write>(
    builder: &mut Builder<W>,
    dest: &Path,
    data: &[u8],
    mode: u32,
) -> Result<()> {
    let mut header = Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(mode);
    header.set_cksum();

    builder
        .append_data(&mut header, dest, Cursor::new(data))
        .with_context(|| format!("failed to append {}", dest.display()))?;

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
