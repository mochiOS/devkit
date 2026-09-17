use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use mochios_certificate::DeveloperCertificate;

use crate::cli::{IdentityImportArgs, PackageSignArgs, PackageSignAutoArgs};
use crate::commands::mpkg;
use crate::crypto;

const IDENTITIES_DIR: &str = "identities";
const CERTIFICATE_FILE: &str = "developer.cert";
const PRIVATE_KEY_FILE: &str = "private.key";
const DEFAULT_FILE: &str = "default";

struct StoredIdentity {
    name: String,
    certificate_path: PathBuf,
    key_path: PathBuf,
    certificate: DeveloperCertificate,
}

pub fn import(args: IdentityImportArgs) -> Result<()> {
    validate_selector(&args.name)?;
    let root = store_root()?;
    ensure_private_directory(&root)?;
    let identities = root.join(IDENTITIES_DIR);
    ensure_private_directory(&identities)?;
    let destination = identities.join(&args.name);
    if destination.exists() {
        bail!("signing identity already exists: {}", args.name);
    }

    let certificate_bytes = fs::read(&args.certificate)
        .with_context(|| format!("failed to read {}", args.certificate.display()))?;
    let certificate = mpkg::decode_canonical_certificate(&certificate_bytes)?;
    let private_key = crypto::read_private_key(&args.key)?;
    if private_key.verifying_key().to_bytes() != certificate.subject_public_key {
        bail!("private key does not match certificate subject public key");
    }

    ensure_private_directory(&destination)?;
    let key_path = destination.join(PRIVATE_KEY_FILE);
    let certificate_path = destination.join(CERTIFICATE_FILE);
    if let Err(error) = crypto::write_private_key(&key_path, &private_key) {
        let _ = fs::remove_dir_all(&destination);
        return Err(error);
    }
    if let Err(error) = write_new_file(&certificate_path, &certificate_bytes, 0o644) {
        let _ = fs::remove_dir_all(&destination);
        return Err(error);
    }
    if args.default {
        write_default_selector(&root, &args.name)?;
    }

    println!("imported: {}", args.name);
    println!("developer_id: {}", certificate.developer_id);
    println!("store: {}", destination.display());
    Ok(())
}

pub fn list() -> Result<()> {
    let root = store_root()?;
    let default = read_default_selector(&root)?;
    let mut identities = load_identities(&root)?;
    identities.sort_by(|left, right| left.name.cmp(&right.name));
    for identity in identities {
        let marker = if default.as_deref() == Some(identity.name.as_str()) {
            " default"
        } else {
            ""
        };
        println!(
            "{}{} developer_id={} subject_key_id={}",
            identity.name,
            marker,
            identity.certificate.developer_id,
            hex(&identity.certificate.subject_key_id)
        );
    }
    Ok(())
}

pub fn sign_auto(args: PackageSignAutoArgs) -> Result<()> {
    let root = store_root()?;
    let requirements = mpkg::package_requirements(&args.package)?;
    let now = match args.unix_time {
        Some(value) => value,
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .context("system time is before UNIX_EPOCH")?
            .as_secs(),
    };
    let identities = load_identities(&root)?;
    let identity = if let Some(selector) = args.identity.as_deref() {
        validate_selector(selector)?;
        identities
            .into_iter()
            .find(|identity| identity.name == selector)
            .ok_or_else(|| anyhow!("signing identity not found: {selector}"))?
    } else {
        select_identity(&root, identities, &requirements, now)?
    };

    println!("identity: {}", identity.name);
    mpkg::sign(PackageSignArgs {
        package: args.package,
        certificate: identity.certificate_path,
        key: identity.key_path,
        output: args.output,
        unix_time: Some(now),
        replace_signature: args.replace_signature,
    })
}

fn select_identity(
    root: &Path,
    identities: Vec<StoredIdentity>,
    requirements: &mpkg::PackageRequirements,
    now: u64,
) -> Result<StoredIdentity> {
    let default = read_default_selector(root)?;
    let mut eligible = identities
        .into_iter()
        .filter(|identity| {
            let certificate = &identity.certificate;
            now >= certificate.not_before
                && now < certificate.not_after
                && certificate
                    .package_id_scopes
                    .iter()
                    .any(|scope| scope.matches(&requirements.package_id))
                && requirements.capabilities.iter().all(|requested| {
                    certificate
                        .allowed_capabilities
                        .iter()
                        .any(|allowed| allowed == requested)
                })
        })
        .collect::<Vec<_>>();
    if let Some(default) = default {
        if let Some(index) = eligible
            .iter()
            .position(|identity| identity.name == default)
        {
            return Ok(eligible.swap_remove(index));
        }
    }
    match eligible.len() {
        0 => bail!(
            "no local signing identity covers package {} and its requested capabilities",
            requirements.package_id
        ),
        1 => Ok(eligible.pop().unwrap()),
        _ => {
            let names = eligible
                .iter()
                .map(|identity| identity.name.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("multiple signing identities match ({names}); select one with --identity")
        }
    }
}

fn load_identities(root: &Path) -> Result<Vec<StoredIdentity>> {
    let identities_path = root.join(IDENTITIES_DIR);
    let entries = match fs::read_dir(&identities_path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).context("failed to read signing identity store"),
    };
    let mut identities = Vec::new();
    for entry in entries {
        let entry = entry.context("failed to read signing identity entry")?;
        let file_type = entry.file_type().context("failed to inspect signing identity")?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        validate_selector(&name)?;
        let certificate_path = entry.path().join(CERTIFICATE_FILE);
        let key_path = entry.path().join(PRIVATE_KEY_FILE);
        reject_symlink(&certificate_path)?;
        reject_symlink(&key_path)?;
        let certificate_bytes = fs::read(&certificate_path)
            .with_context(|| format!("failed to read {}", certificate_path.display()))?;
        let certificate = mpkg::decode_canonical_certificate(&certificate_bytes)?;
        let key = crypto::read_private_key(&key_path)?;
        if key.verifying_key().to_bytes() != certificate.subject_public_key {
            bail!("stored identity {name} has a mismatched private key");
        }
        identities.push(StoredIdentity {
            name,
            certificate_path,
            key_path,
            certificate,
        });
    }
    Ok(identities)
}

fn store_root() -> Result<PathBuf> {
    if let Some(path) = env::var_os("MOCHIOS_SIGNING_HOME") {
        return nonempty_path(path, "MOCHIOS_SIGNING_HOME");
    }
    if let Some(path) = env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(path).join("mochios/signing"));
    }
    let home = env::var_os("HOME").ok_or_else(|| {
        anyhow!("HOME is not set; set MOCHIOS_SIGNING_HOME to the identity store path")
    })?;
    Ok(PathBuf::from(home).join(".config/mochios/signing"))
}

fn nonempty_path(value: std::ffi::OsString, name: &str) -> Result<PathBuf> {
    if value.is_empty() {
        bail!("{name} must not be empty");
    }
    Ok(PathBuf::from(value))
}

fn validate_selector(selector: &str) -> Result<()> {
    if selector.is_empty()
        || selector.len() > 64
        || !selector
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("identity name must contain only ASCII letters, digits, '-' or '_'");
    }
    Ok(())
}

fn ensure_private_directory(path: &Path) -> Result<()> {
    if path.exists() {
        reject_symlink(path)?;
        if !path.is_dir() {
            bail!("identity store path is not a directory: {}", path.display());
        }
    } else {
        fs::create_dir_all(path)
            .with_context(|| format!("failed to create {}", path.display()))?;
    }
    set_directory_mode(path, 0o700)
}

fn reject_symlink(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("refusing symbolic link in identity store: {}", path.display());
    }
    Ok(())
}

fn read_default_selector(root: &Path) -> Result<Option<String>> {
    let path = root.join(DEFAULT_FILE);
    let value = match fs::read_to_string(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("failed to read default signing identity"),
    };
    let value = value.trim().to_string();
    validate_selector(&value)?;
    Ok(Some(value))
}

fn write_default_selector(root: &Path, selector: &str) -> Result<()> {
    let temporary = root.join("default.new");
    if temporary.exists() {
        fs::remove_file(&temporary).context("failed to remove stale default.new")?;
    }
    write_new_file(&temporary, selector.as_bytes(), 0o600)?;
    fs::rename(&temporary, root.join(DEFAULT_FILE))
        .context("failed to replace default signing identity")
}

fn write_new_file(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    open_with_mode(&mut options, mode);
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    use std::io::Write;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(unix)]
fn set_directory_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .with_context(|| format!("failed to secure {}", path.display()))
}

#[cfg(not(unix))]
fn set_directory_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn open_with_mode(options: &mut fs::OpenOptions, mode: u32) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(mode);
}

#[cfg(not(unix))]
fn open_with_mode(_options: &mut fs::OpenOptions, _mode: u32) {}

fn hex(bytes: &[u8]) -> String {
    const TABLE: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(TABLE[(byte >> 4) as usize] as char);
        output.push(TABLE[(byte & 0x0f) as usize] as char);
    }
    output
}
