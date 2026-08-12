use std::fs;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use crate::cli::{CreateArgs, LinuxArgs};

pub fn create(args: LinuxArgs) -> Result<()> {
    validate(&args)?;
    let temporary = tempfile::tempdir().context("failed to create Linux package workspace")?;
    let rootfs = temporary.path().join("rootfs");
    fs::create_dir_all(&rootfs)?;

    if let Some(package) = &args.source.apt_package {
        populate_from_apt(&temporary, &rootfs, package, &args.architecture)?;
    } else if let Some(binary) = &args.source.linux_binary {
        populate_from_binary(&rootfs, binary, &args.entrypoint)?;
    } else if let Some(source) = &args.source.rootfs {
        copy_rootfs(source, &rootfs)?;
    } else {
        bail!("one Linux package source is required");
    }

    create_runtime_directories(&rootfs)?;
    let entry = rootfs.join(args.entrypoint.trim_start_matches('/'));
    if !entry.is_file() {
        bail!(
            "Linux entrypoint is missing from rootfs: {}",
            entry.display()
        );
    }

    let payload = temporary.path().join("payload/bundle");
    fs::create_dir_all(&payload)?;
    let squashfs = payload.join("rootfs.squashfs");
    run(
        Command::new("mksquashfs")
            .arg(&rootfs)
            .arg(&squashfs)
            .args([
                "-noappend",
                "-all-root",
                "-no-xattrs",
                "-no-progress",
                "-comp",
                "gzip",
                "-mkfs-time",
                "0",
                "-all-time",
                "0",
            ]),
        "mksquashfs",
    )?;

    let about = about_toml(&args);
    fs::write(payload.join("about.toml"), about.as_bytes())?;
    if let Some(icon) = &args.icon {
        fs::copy(icon, payload.join("appicon.png"))
            .with_context(|| format!("failed to copy {}", icon.display()))?;
    }

    let manifest = manifest_toml(&args, &payload)?;
    let manifest_path = temporary.path().join("manifest.toml");
    fs::write(&manifest_path, manifest)?;
    crate::mpkg::create(CreateArgs {
        manifest: manifest_path,
        payload: temporary.path().join("payload"),
        output: args.output,
        force: args.force,
    })
}

fn validate(args: &LinuxArgs) -> Result<()> {
    if !valid_bundle_id(&args.bundle_id) {
        bail!("invalid Linux application bundle ID");
    }
    if args.name.is_empty()
        || args.name.contains('/')
        || args.name.contains('\\')
        || args.version.is_empty()
        || args.vendor.is_empty()
    {
        bail!("name, version, or vendor is invalid");
    }
    if !valid_absolute_path(&args.entrypoint) {
        bail!("entrypoint must be a normalized absolute Linux path");
    }
    if !matches!(args.architecture.as_str(), "amd64" | "x86_64") {
        bail!("only amd64/x86_64 Linux packages are supported");
    }
    for path in &args.writable_paths {
        if !valid_writable_path(path) {
            bail!("unsafe writable Linux path: {path}");
        }
    }
    if args.writable_paths.iter().enumerate().any(|(index, path)| {
        args.writable_paths[index + 1..]
            .iter()
            .any(|other| paths_overlap(path, other))
    }) {
        bail!("Linux writable paths must not overlap");
    }
    if let Some(package) = &args.source.apt_package {
        if package.is_empty()
            || package.starts_with('-')
            || !package.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-' | b':' | b'=')
            })
        {
            bail!("invalid APT package specification");
        }
    }
    Ok(())
}

fn populate_from_apt(
    temporary: &TempDir,
    rootfs: &Path,
    package: &str,
    architecture: &str,
) -> Result<()> {
    let state = temporary.path().join("apt-state");
    let archives = temporary.path().join("apt-archives");
    fs::create_dir_all(archives.join("partial"))?;
    fs::create_dir_all(&state)?;
    let status = state.join("status");
    fs::write(&status, [])?;
    let apt_arch = if architecture == "x86_64" {
        "amd64"
    } else {
        architecture
    };
    run(
        Command::new("apt-get")
            .arg("-y")
            .arg("--download-only")
            .arg("--no-install-recommends")
            .arg("-o")
            .arg(format!("Dir::State::status={}", status.display()))
            .arg("-o")
            .arg(format!("Dir::Cache::archives={}", archives.display()))
            .arg("-o")
            .arg("Debug::NoLocking=1")
            .arg("-o")
            .arg(format!("APT::Architecture={apt_arch}"))
            .arg("install")
            .arg("--")
            .arg(package),
        "apt-get dependency download",
    )?;
    let mut packages = fs::read_dir(&archives)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension == "deb"))
        .collect::<Vec<_>>();
    packages.sort();
    if packages.is_empty() {
        bail!("APT did not download any packages");
    }
    for package_path in packages {
        run(
            Command::new("dpkg-deb")
                .arg("--extract")
                .arg(&package_path)
                .arg(rootfs),
            "dpkg-deb extraction",
        )?;
    }
    Ok(())
}

fn populate_from_binary(rootfs: &Path, binary: &Path, entrypoint: &str) -> Result<()> {
    if !binary.is_file() {
        bail!("Linux binary does not exist: {}", binary.display());
    }
    let target = rootfs.join(entrypoint.trim_start_matches('/'));
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(binary, &target).with_context(|| format!("failed to copy {}", binary.display()))?;
    set_mode(&target, 0o755)?;
    Ok(())
}

fn copy_rootfs(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_dir() {
        bail!("Linux rootfs does not exist: {}", source.display());
    }
    run(
        Command::new("cp")
            .args(["-a", "--"])
            .arg(format!("{}/.", source.display()))
            .arg(destination),
        "rootfs copy",
    )
}

fn create_runtime_directories(rootfs: &Path) -> Result<()> {
    for path in ["dev", "proc", "tmp", "var/tmp", "home/user", "mochios"] {
        fs::create_dir_all(rootfs.join(path))?;
    }
    set_mode(&rootfs.join("tmp"), 0o1777)?;
    set_mode(&rootfs.join("var/tmp"), 0o1777)?;
    Ok(())
}

fn about_toml(args: &LinuxArgs) -> String {
    format!(
        "name = {name:?}\nbundle-id = {bundle:?}\nversion = {version:?}\ndeveloper = {vendor:?}\nentry = {entry:?}\ndescription = \"Linux application hosted by mBoot\"\nicon = {icon:?}\n",
        name = args.name,
        bundle = args.bundle_id,
        version = args.version,
        vendor = args.vendor,
        entry = format!("linux:{}", args.bundle_id),
        icon = args.icon.as_ref().map_or("", |_| "appicon.png"),
    )
}

fn manifest_toml(args: &LinuxArgs, payload: &Path) -> Result<String> {
    let rootfs = file_record(
        "linux-rootfs",
        "$/rootfs.squashfs",
        &payload.join("rootfs.squashfs"),
    )?;
    let about = file_record("about", "$/about.toml", &payload.join("about.toml"))?;
    let icon = match &args.icon {
        Some(_) => file_record("icon", "$/appicon.png", &payload.join("appicon.png"))?,
        None => String::new(),
    };
    let writable = args
        .writable_paths
        .iter()
        .map(|path| format!("    {path:?},\n"))
        .collect::<String>();
    Ok(format!(
        "format = 1\n\n[package]\nid = {bundle:?}\nname = {name:?}\nversion = {version:?}\nvendor = {vendor:?}\nkind = \"application\"\narchitecture = \"x86_64\"\nabi = \"mboot-linux-1\"\n\n[linux]\nentrypoint = {entrypoint:?}\nrootfs_file = \"linux-rootfs\"\nwritable_paths = [\n{writable}]\n\n{rootfs}{about}{icon}",
        bundle = args.bundle_id,
        name = args.name,
        version = args.version,
        vendor = args.vendor,
        entrypoint = args.entrypoint,
    ))
}

fn file_record(id: &str, path: &str, source: &Path) -> Result<String> {
    let bytes = fs::read(source)?;
    let digest = hex(&Sha256::digest(&bytes));
    Ok(format!(
        "[[file]]\nid = {id:?}\npath = {path:?}\ndigest = \"sha256:{digest}\"\nsize = {}\nmode = \"0644\"\n\n",
        bytes.len()
    ))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn run(command: &mut Command, operation: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("failed to execute {operation}"))?;
    if !status.success() {
        bail!("{operation} failed with {status}");
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

fn valid_bundle_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-'))
}

fn valid_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        && path.len() > 1
        && !path.ends_with('/')
        && !path.contains("//")
        && path[1..]
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn valid_writable_path(path: &str) -> bool {
    valid_absolute_path(path)
        && !["/dev", "/proc", "/sys", "/run", "/tmp", "/home", "/mochios"]
            .iter()
            .any(|root| path == *root || path.starts_with(&format!("{root}/")))
        && !matches!(
            path,
            "/bin" | "/sbin" | "/lib" | "/lib64" | "/usr" | "/usr/bin" | "/usr/lib"
        )
        && !path.starts_with("/bin/")
        && !path.starts_with("/sbin/")
        && !path.starts_with("/lib/")
        && !path.starts_with("/lib64/")
        && !path.starts_with("/usr/bin/")
        && !path.starts_with("/usr/lib/")
        && !path.starts_with("/mochios")
}

fn paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::LinuxSource;

    fn args() -> LinuxArgs {
        LinuxArgs {
            bundle_id: String::from("org.example.editor"),
            name: String::from("Editor"),
            version: String::from("1"),
            vendor: String::from("Example"),
            entrypoint: String::from("/usr/bin/editor"),
            source: LinuxSource {
                apt_package: None,
                linux_binary: Some(PathBuf::from("editor")),
                rootfs: None,
            },
            architecture: String::from("amd64"),
            writable_paths: vec![String::from("/usr/share/editor")],
            icon: None,
            output: PathBuf::from("editor.mpkg"),
            force: false,
        }
    }

    #[test]
    fn validates_identity_and_writable_boundaries() {
        assert!(validate(&args()).is_ok());
        let mut unsafe_args = args();
        unsafe_args.writable_paths = vec![String::from("/usr/bin")];
        assert!(validate(&unsafe_args).is_err());
        unsafe_args = args();
        unsafe_args.bundle_id = String::from("../editor");
        assert!(validate(&unsafe_args).is_err());
        unsafe_args = args();
        unsafe_args.writable_paths = vec![String::from("/home/user")];
        assert!(validate(&unsafe_args).is_err());
        unsafe_args = args();
        unsafe_args.writable_paths = vec![
            String::from("/var/lib/editor"),
            String::from("/var/lib/editor/cache"),
        ];
        assert!(validate(&unsafe_args).is_err());
    }
}
