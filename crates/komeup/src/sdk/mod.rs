//! C/C++ SDK lifecycle management, separate from compiler builds and project configuration.
mod archive;
mod source;
mod store;

use anyhow::{bail, Context, Result};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

const TARGET: &str = "x86_64-unknown-mochios";
const ABI: &str = "mochios-1";
const HOST: &str = "x86_64-linux-gnu-ubuntu24.04";
const RECEIPT: &str = ".komeup-sdk.toml";
const INSTALLED_MANIFEST: &str = ".komeup-sdk-manifest.json";
const MAX_ARCHIVE: u64 = 2 * 1024 * 1024 * 1024;
const MAX_EXPANDED: u64 = 8 * 1024 * 1024 * 1024;

/// Explicit SDK lifecycle operations; versions are never inferred from AppCore releases.
#[derive(Debug, Subcommand)]
pub enum SdkCommand {
    /// Install a published version without replacing other installed versions.
    Install {
        /// SDK version, for example 27.0-dp.1-shared.2 (without the sdk- tag prefix).
        version: String,
        /// Read the archive, JSON manifest and SHA256SUMS from a local directory.
        #[arg(long = "from", value_name = "DIRECTORY")]
        from: Option<PathBuf>,
        /// Select this version as the default after successful verification.
        #[arg(long)]
        default: bool,
    },
    /// Install an explicit new version and select it, keeping previous versions for rollback.
    Update {
        version: String,
        #[arg(long = "from", value_name = "DIRECTORY")]
        from: Option<PathBuf>,
    },
    /// List managed installations and the selected default.
    List,
    /// Select an already installed SDK; project-pinned CMake paths remain unchanged.
    Default { version: String },
    /// Print an immutable installed SDK path for build-system configuration.
    Path {
        #[arg(long)]
        version: Option<String>,
        /// Print the SDK's CMake toolchain file instead of its root.
        #[arg(long, conflicts_with = "bin")]
        cmake: bool,
        /// Print the SDK's compiler directory instead of its root.
        #[arg(long)]
        bin: bool,
    },
    /// Verify installed file hashes and check host prerequisites. Does not execute SDK binaries.
    Doctor {
        #[arg(long)]
        version: Option<String>,
    },
    /// Remove one managed SDK version. The Kome toolchain and other SDKs are preserved.
    Uninstall { version: String },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
struct SdkInfo {
    format: u32,
    version: String,
    target: String,
    abi: String,
    host: String,
    features: Vec<String>,
    sysroot: String,
    cmake_toolchain: String,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    format: u32,
    sdk: serde_json::Value,
    archive: String,
    sha256: String,
    files: BTreeMap<String, FileRecord>,
}

#[derive(Debug, Deserialize)]
struct FileRecord {
    sha256: Option<String>,
    size: Option<u64>,
    symlink: Option<String>,
}

/// Run a namespaced SDK command without changing Kome/AppCore selection or publishing artifacts.
pub fn run(command: SdkCommand) -> Result<()> {
    let store = store::Store::open(crate::config::kome_home()?)?;
    match command {
        SdkCommand::Install {
            version,
            from,
            default,
        } => install(&store, &version, from.as_deref(), default),
        SdkCommand::Update { version, from } => install(&store, &version, from.as_deref(), true),
        SdkCommand::List => store.list(),
        SdkCommand::Default { version } => {
            version_check(&version)?;
            store.activate(&version)?;
            println!("Default C/C++ SDK: {version}");
            Ok(())
        }
        SdkCommand::Path {
            version,
            cmake,
            bin,
        } => {
            let (root, info) = store.selected(version.as_deref())?;
            let path = if cmake {
                root.join(relative(&info.cmake_toolchain)?)
            } else if bin {
                root.join("bin")
            } else {
                root
            };
            println!("{}", path.display());
            Ok(())
        }
        SdkCommand::Doctor { version } => store.doctor(version.as_deref()),
        SdkCommand::Uninstall { version } => {
            version_check(&version)?;
            store.uninstall(&version)
        }
    }
}

fn install(store: &store::Store, version: &str, from: Option<&Path>, select: bool) -> Result<()> {
    supported_host()?;
    version_check(version)?;
    eprintln!("Fetching C/C++ SDK {version}...");
    let source = source::Source::load(version, from)?;
    let stage = store.stage()?;
    let archive = stage.path().join("download.tar.gz");
    source.download_archive(&archive)?;
    let root = stage.path().join("payload");
    std::fs::create_dir(&root)?;
    eprintln!("Verifying SDK files and extracting into private staging...");
    archive::extract(&archive, &root, &source.manifest)?;
    let info = validate_sdk(&root, version, &source.manifest)?;
    let receipt = store::Receipt::new(version, &source.origin, &source.manifest.sha256)?;
    let installed = store.commit(&root, &source.manifest_bytes, &receipt)?;
    store
        .select_after_install(version, select)
        .with_context(|| {
            format!(
                "SDK is installed at {}, but could not be selected",
                installed.display()
            )
        })?;
    println!("Installed C/C++ SDK {} ({})", info.version, info.target);
    println!("SDK: {}", installed.display());
    println!("Compiler shims: {}", store.home.join("bin").display());
    println!("CMake: komeup sdk path --version {version} --cmake");
    Ok(())
}

fn supported_host() -> Result<()> {
    if std::env::consts::OS != "linux" || std::env::consts::ARCH != "x86_64" {
        bail!("This SDK currently requires x86-64 Linux compatible with Ubuntu 24.04");
    }
    Ok(())
}

fn version_check(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || value.contains("..")
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || b"._+-".contains(&ch))
    {
        bail!("Invalid SDK version: {value}");
    }
    Ok(())
}

fn relative(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\\')
        || value.contains(':')
        || value.chars().any(char::is_control)
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("Invalid SDK-relative path: {value}");
    }
    Ok(path.to_path_buf())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
}

fn validate_sdk(root: &Path, version: &str, manifest: &Manifest) -> Result<SdkInfo> {
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("sdk.json"))?)?;
    if value != manifest.sdk {
        bail!("Embedded SDK metadata disagrees with the release manifest");
    }
    let info: SdkInfo = serde_json::from_value(value)?;
    if info.format != 1
        || info.version != version
        || info.target != TARGET
        || info.abi != ABI
        || info.host != HOST
    {
        bail!("SDK identity, target, ABI or host is incompatible");
    }
    for feature in ["pic", "shared", "cxx23", "threads", "exceptions"] {
        if !info.features.iter().any(|item| item == feature) {
            bail!("SDK feature is missing: {feature}");
        }
    }
    if !root.join(relative(&info.sysroot)?).is_dir()
        || !root.join(relative(&info.cmake_toolchain)?).is_file()
    {
        bail!("SDK sysroot or CMake toolchain is missing");
    }
    for tool in [
        "bin/mochios-cc",
        "bin/mochios-c++",
        "host/bin/clang",
        "host/bin/ld.lld",
        "host/bin/ar",
        "host/bin/ranlib",
    ] {
        let path = root.join(tool);
        if !path.is_file() || !path.canonicalize()?.starts_with(root.canonicalize()?) {
            bail!("SDK tool is missing or external: {tool}");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if std::fs::metadata(path)?.permissions().mode() & 0o100 == 0 {
                bail!("SDK tool is not executable: {tool}");
            }
        }
    }
    Ok(info)
}

#[cfg(test)]
mod tests;
