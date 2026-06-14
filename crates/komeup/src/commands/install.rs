use crate::config::{
    kome_home, load_or_create_config, path_string, save_config, ComponentInfo, ToolchainInfo,
};
use crate::github;
use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use std::collections::BTreeMap;
use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tar::Archive;

#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};

const CHANNEL_STABLE: &str = "stable";

const REPO_KOMEC: &str = "komec";
const REPO_STD: &str = "kome_std";
const REPO_DEVKIT: &str = "devkit";

const KOMEC_ASSET: &str = "x86_64-linux-komec-stable";

const DEVKIT_ASSETS: &[(&str, &str)] = &[
    ("kome", "x86_64-linux-kome-stable"),
    ("msign", "x86_64-linux-msign-stable"),
    ("mpack", "x86_64-linux-mpack-stable"),
];

const SHIM_BINARIES: &[&str] = &[
    "kome",
    "komec",
    "msign",
    "mpack",
];

pub fn run(channel: &str, force: bool) -> Result<()> {
    ensure_supported_channel(channel)?;

    let home = kome_home()?;
    let toolchain_dir = home.join("toolchains").join(channel);
    let bin_dir = toolchain_dir.join("bin");
    let lib_dir = toolchain_dir.join("lib");
    let std_dir = lib_dir.join("std");

    if toolchain_dir.exists() && !force {
        bail!(
            "toolchain '{}' is already installed. Use --force or `komeup update`.",
            channel
        );
    }

    fs::create_dir_all(&bin_dir)
        .with_context(|| format!("failed to create {}", bin_dir.display()))?;

    fs::create_dir_all(&lib_dir)
        .with_context(|| format!("failed to create {}", lib_dir.display()))?;

    let client = github::client()?;

    let komec_release = github::latest_release(&client, REPO_KOMEC)?;
    let std_release = github::latest_release(&client, REPO_STD)?;
    let devkit_release = github::latest_release(&client, REPO_DEVKIT)?;

    let mut components = BTreeMap::new();

    install_komec(
        &client,
        &komec_release,
        &bin_dir,
        &mut components,
    )?;

    install_std(
        &client,
        &std_release,
        &std_dir,
        &mut components,
    )?;

    install_devkit(
        &client,
        &devkit_release,
        &bin_dir,
        &mut components,
    )?;

    let mut config = load_or_create_config(&home)?;
    config.default_toolchain = channel.to_string();

    config.toolchains.insert(
        channel.to_string(),
        ToolchainInfo {
            channel: channel.to_string(),
            version: komec_release.tag_name.clone(),
            path: path_string(&toolchain_dir),
            installed_at: now_unix_timestamp()?,
            components,
        },
    );

    save_config(&home, &config)?;
    refresh_shims(&home, channel)?;

    println!("installed Kome toolchain '{}'", channel);
    println!("home: {}", home.display());
    println!("bin:  {}", home.join("bin").display());

    Ok(())
}

fn install_komec(
    client: &Client,
    release: &github::GithubRelease,
    bin_dir: &Path,
    components: &mut BTreeMap<String, ComponentInfo>,
) -> Result<()> {
    let asset = github::find_asset_exact(release, KOMEC_ASSET)?;
    let out_path = bin_dir.join("komec");

    install_binary(client, &asset.browser_download_url, &out_path)?;

    components.insert(
        "komec".to_string(),
        ComponentInfo {
            name: "komec".to_string(),
            version: release.tag_name.clone(),
            path: path_string(out_path),
            source: asset.browser_download_url,
        },
    );

    Ok(())
}

fn install_std(
    client: &Client,
    release: &github::GithubRelease,
    std_dir: &Path,
    components: &mut BTreeMap<String, ComponentInfo>,
) -> Result<()> {
    if std_dir.exists() {
        fs::remove_dir_all(std_dir)
            .with_context(|| format!("failed to remove {}", std_dir.display()))?;
    }

    fs::create_dir_all(std_dir)
        .with_context(|| format!("failed to create {}", std_dir.display()))?;

    let asset = github::find_std_asset(release)?;

    install_tar_gz(client, &asset.browser_download_url, std_dir)?;

    components.insert(
        "std".to_string(),
        ComponentInfo {
            name: "std".to_string(),
            version: release.tag_name.clone(),
            path: path_string(std_dir),
            source: asset.browser_download_url,
        },
    );

    Ok(())
}

fn install_devkit(
    client: &Client,
    release: &github::GithubRelease,
    bin_dir: &Path,
    components: &mut BTreeMap<String, ComponentInfo>,
) -> Result<()> {
    for (binary_name, asset_name) in DEVKIT_ASSETS {
        let asset = github::find_asset_exact(release, asset_name)?;
        let out_path = bin_dir.join(binary_name);

        install_binary(client, &asset.browser_download_url, &out_path)?;

        components.insert(
            (*binary_name).to_string(),
            ComponentInfo {
                name: (*binary_name).to_string(),
                version: release.tag_name.clone(),
                path: path_string(out_path),
                source: asset.browser_download_url,
            },
        );
    }

    Ok(())
}

fn install_binary(client: &Client, url: &str, out_path: &Path) -> Result<()> {
    let bytes = github::download_bytes(client, url)?;

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    fs::write(out_path, bytes)
        .with_context(|| format!("failed to write {}", out_path.display()))?;

    make_executable(out_path)?;

    Ok(())
}

fn install_tar_gz(client: &Client, url: &str, out_dir: &Path) -> Result<()> {
    let bytes = github::download_bytes(client, url)?;

    let cursor = Cursor::new(bytes);
    let decoder = GzDecoder::new(cursor);
    let mut archive = Archive::new(decoder);

    archive
        .unpack(out_dir)
        .with_context(|| format!("failed to unpack archive into {}", out_dir.display()))?;

    Ok(())
}

fn refresh_shims(home: &Path, channel: &str) -> Result<()> {
    let shim_dir = home.join("bin");
    let toolchain_bin = home.join("toolchains").join(channel).join("bin");

    fs::create_dir_all(&shim_dir)
        .with_context(|| format!("failed to create {}", shim_dir.display()))?;

    for binary in SHIM_BINARIES {
        let src = toolchain_bin.join(binary);
        let dst = shim_dir.join(binary);

        if dst.exists() || dst.is_symlink() {
            fs::remove_file(&dst)
                .with_context(|| format!("failed to remove {}", dst.display()))?;
        }

        create_symlink(&src, &dst)?;
    }

    Ok(())
}

#[cfg(unix)]
fn create_symlink(src: &Path, dst: &Path) -> Result<()> {
    symlink(src, dst)
        .with_context(|| format!("failed to create symlink {} -> {}", dst.display(), src.display()))?;

    Ok(())
}

#[cfg(not(unix))]
fn create_symlink(_src: &Path, _dst: &Path) -> Result<()> {
    bail!("komeup currently supports Unix-like systems only")
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    let mut perms = fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .permissions();

    perms.set_mode(0o755);

    fs::set_permissions(path, perms)
        .with_context(|| format!("failed to chmod {}", path.display()))?;

    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    bail!("komeup currently supports Unix-like systems only")
}

fn ensure_supported_channel(channel: &str) -> Result<()> {
    if channel != CHANNEL_STABLE {
        bail!("unsupported channel '{}'. currently only 'stable' is supported", channel);
    }

    Ok(())
}

fn now_unix_timestamp() -> Result<String> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system time is before UNIX_EPOCH")?
        .as_secs();

    Ok(seconds.to_string())
}

pub fn refresh_default_shims(home: &Path, channel: &str) -> Result<()> {
    refresh_shims(home, channel)
}