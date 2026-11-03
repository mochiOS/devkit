//! Immutable SDK versions and atomic default selection; no project configuration is rewritten.
use super::{archive, validate_sdk, version_check, Manifest, SdkInfo, INSTALLED_MANIFEST, RECEIPT};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) struct Store {
    pub home: PathBuf,
    root: PathBuf,
}

#[derive(Deserialize, Serialize)]
pub(super) struct Receipt {
    format: u32,
    version: String,
    source: String,
    sha256: String,
    installed_at: u64,
}
impl Receipt {
    /// Store local installation provenance independently of the Kome language toolchain configuration.
    pub fn new(version: &str, source: &str, sha256: &str) -> Result<Self> {
        Ok(Self {
            format: 1,
            version: version.into(),
            source: source.into(),
            sha256: sha256.into(),
            installed_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("System clock is before UNIX_EPOCH")?
                .as_secs(),
        })
    }
}

impl Store {
    /// Open the SDK namespace under the selected KOMEUP_HOME; do not create or modify a Kome toolchain.
    pub fn open(home: PathBuf) -> Result<Self> {
        let home = if home.is_absolute() {
            home
        } else {
            std::env::current_dir()?.join(home)
        };
        Ok(Self {
            root: home.join("sdks"),
            home,
        })
    }

    fn initialize(&self) -> Result<()> {
        fs::create_dir_all(&self.home)?;
        for path in [
            &self.root,
            self.root.join("installed").as_path(),
            self.root.join("staging").as_path(),
        ] {
            #[cfg(unix)]
            let created = {
                use std::os::unix::fs::DirBuilderExt;
                fs::DirBuilder::new().mode(0o700).create(path)
            };
            #[cfg(not(unix))]
            let created = fs::create_dir(path);
            if let Err(error) = created {
                if error.kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(error.into());
                }
            }
            let metadata = fs::symlink_metadata(path)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                bail!(
                    "SDK manager directory is not a regular directory: {}",
                    path.display()
                );
            }
        }
        Ok(())
    }

    fn lock(&self) -> Result<File> {
        self.initialize()?;
        let path = self.root.join("manager.lock");
        if path.is_symlink() {
            bail!("SDK lock cannot be a symlink");
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.lock().context("Cannot lock SDK manager")?;
        Ok(file)
    }

    /// Create private staging on the installation filesystem so committing a version is one rename.
    pub fn stage(&self) -> Result<tempfile::TempDir> {
        self.initialize()?;
        Ok(tempfile::Builder::new()
            .prefix("install-")
            .tempdir_in(self.root.join("staging"))?)
    }

    fn version_root(&self, version: &str) -> Result<PathBuf> {
        version_check(version)?;
        Ok(self.root.join("installed").join(version))
    }

    fn receipt(&self, version: &str) -> Result<Receipt> {
        let root = self.version_root(version)?;
        if fs::symlink_metadata(&root)?.file_type().is_symlink() {
            bail!("SDK version directory cannot be a symlink");
        }
        let receipt: Receipt = toml::from_str(&fs::read_to_string(root.join(RECEIPT))?)?;
        if receipt.format != 1 || receipt.version != version || !super::valid_hash(&receipt.sha256)
        {
            bail!("Invalid SDK installation receipt");
        }
        Ok(receipt)
    }

    /// Commit only a completely verified SDK. Existing immutable versions are never overwritten.
    pub fn commit(&self, payload: &Path, manifest: &[u8], receipt: &Receipt) -> Result<PathBuf> {
        let _lock = self.lock()?;
        let destination = self.version_root(&receipt.version)?;
        if destination.exists() || destination.is_symlink() {
            let installed = self.receipt(&receipt.version)?;
            if installed.sha256 != receipt.sha256 {
                bail!("SDK version already exists with different bytes; use a new SDK version");
            }
            let installed_manifest: Manifest =
                serde_json::from_slice(&fs::read(destination.join(INSTALLED_MANIFEST))?)?;
            archive::verify_installed(&destination, &installed_manifest)?;
            return Ok(destination);
        }
        fs::write(payload.join(INSTALLED_MANIFEST), manifest)?;
        fs::write(payload.join(RECEIPT), toml::to_string_pretty(receipt)?)?;
        fs::rename(payload, &destination).context("Cannot commit SDK installation")?;
        Ok(destination)
    }

    /// Read the single default selector; a regular file or external link is never treated as manager state.
    pub fn current(&self) -> Result<Option<String>> {
        let path = self.root.join("current");
        let link = match fs::read_link(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !path.is_symlink() => {
                return Ok(None)
            }
            Err(error) => {
                return Err(error).context("SDK current selector is not a managed symlink")
            }
        };
        let parts: Vec<_> = link.components().collect();
        let [Component::Normal(directory), Component::Normal(version)] = parts.as_slice() else {
            bail!("Invalid SDK default selector");
        };
        if *directory != "installed" {
            bail!("SDK default selector is external");
        }
        let version = version
            .to_str()
            .context("Invalid default SDK version")?
            .to_string();
        version_check(&version)?;
        Ok(Some(version))
    }

    /// Atomically switch one selector; stable compiler shims follow it while CMake version paths stay pinned.
    pub fn activate(&self, version: &str) -> Result<()> {
        self.select_after_install(version, true)
    }

    /// Choose an initial default under the same lock as selection, avoiding concurrent-install races.
    pub fn select_after_install(&self, version: &str, replace: bool) -> Result<()> {
        let _lock = self.lock()?;
        if !replace && self.current()?.is_some() {
            return Ok(());
        }
        self.receipt(version)?;
        let manifest: Manifest = serde_json::from_slice(&fs::read(
            self.version_root(version)?.join(INSTALLED_MANIFEST),
        )?)?;
        validate_sdk(&self.version_root(version)?, version, &manifest)?;
        archive::verify_installed(&self.version_root(version)?, &manifest)?;
        let bin = self.home.join("bin");
        if bin.is_symlink() {
            bail!("Compiler shim directory cannot be a symlink");
        }
        fs::create_dir_all(&bin)?;
        for name in ["mochios-cc", "mochios-c++"] {
            let path = bin.join(name);
            let expected = PathBuf::from(format!("../sdks/current/bin/{name}"));
            if (path.exists() || path.is_symlink())
                && fs::read_link(&path).ok().as_ref() != Some(&expected)
            {
                bail!(
                    "Refusing to replace an unmanaged compiler shim: {}",
                    path.display()
                );
            }
        }
        if self.root.join("current").exists() || self.root.join("current").is_symlink() {
            self.current()?;
        }
        for name in ["mochios-cc", "mochios-c++"] {
            atomic_link(
                &bin.join(name),
                Path::new(&format!("../sdks/current/bin/{name}")),
            )?;
        }
        atomic_link(
            &self.root.join("current"),
            Path::new(&format!("installed/{version}")),
        )?;
        Ok(())
    }

    /// Return one validated installation and metadata without changing the selected version.
    pub fn selected(&self, version: Option<&str>) -> Result<(PathBuf, SdkInfo)> {
        let version = match version {
            Some(value) => value.to_string(),
            None => self
                .current()?
                .context("No C/C++ SDK selected. Run `komeup sdk install VERSION` first")?,
        };
        self.receipt(&version)?;
        let root = self.version_root(&version)?;
        let manifest: Manifest = serde_json::from_slice(&fs::read(root.join(INSTALLED_MANIFEST))?)?;
        let info = validate_sdk(&root, &version, &manifest)?;
        Ok((root.canonicalize()?, info))
    }

    /// List SDK versions independently of whether a Kome compiler has been installed.
    pub fn list(&self) -> Result<()> {
        let current = self.current()?;
        let directory = self.root.join("installed");
        if !directory.exists() {
            println!("No C/C++ SDKs installed");
            return Ok(());
        }
        let mut versions = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Invalid SDK directory name"))?;
            self.receipt(&name)?;
            versions.push(name);
        }
        versions.sort();
        for version in versions {
            println!(
                "{} {}",
                if current.as_deref() == Some(&version) {
                    "*"
                } else {
                    " "
                },
                version
            );
        }
        Ok(())
    }

    /// Check all payload hashes and host Python without running any installed SDK program.
    pub fn doctor(&self, version: Option<&str>) -> Result<()> {
        super::supported_host()?;
        let (root, info) = self.selected(version)?;
        let manifest: Manifest = serde_json::from_slice(&fs::read(root.join(INSTALLED_MANIFEST))?)?;
        archive::verify_installed(&root, &manifest)?;
        let output = std::process::Command::new("python3")
            .args([
                "-c",
                "import sys; sys.exit(0 if sys.version_info >= (3,12) else 1)",
            ])
            .output()
            .context("Python 3.12+ is required by the SDK drivers")?;
        if !output.status.success() {
            bail!("Python 3.12+ is required by the SDK drivers");
        }
        println!(
            "SDK {}: file hashes, links and metadata verified",
            info.version
        );
        println!("Host requirement: Ubuntu 24.04-compatible x86-64 Linux");
        println!("SDK binaries were not executed; mochiOS runtime behavior was not tested");
        Ok(())
    }

    /// Remove only a receipt-bound version and its own default selector, preserving all other toolchains.
    pub fn uninstall(&self, version: &str) -> Result<()> {
        let _lock = self.lock()?;
        self.receipt(version)?;
        if self.current()?.as_deref() == Some(version) {
            fs::remove_file(self.root.join("current"))?;
            for name in ["mochios-cc", "mochios-c++"] {
                let path = self.home.join("bin").join(name);
                if fs::read_link(&path).ok()
                    == Some(PathBuf::from(format!("../sdks/current/bin/{name}")))
                {
                    fs::remove_file(path)?;
                }
            }
        }
        fs::remove_dir_all(self.version_root(version)?)?;
        println!("Removed C/C++ SDK {version}");
        Ok(())
    }
}

fn atomic_link(destination: &Path, target: &Path) -> Result<()> {
    let parent = destination.parent().context("Symlink has no parent")?;
    let temporary = tempfile::Builder::new()
        .prefix("sdk-selector-")
        .tempdir_in(parent)?;
    let link = temporary.path().join("link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, &link)?;
    #[cfg(not(unix))]
    bail!("SDK selection currently requires a Unix-like host");
    fs::rename(link, destination)?;
    Ok(())
}
