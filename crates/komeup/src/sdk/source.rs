//! Bounded downloads from explicit mochiOS SDK releases, or an explicitly selected local directory.
use super::{valid_hash, version_check, Manifest, MAX_ARCHIVE};
use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAX_METADATA: u64 = 8 * 1024 * 1024;

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
}

enum Location {
    Local(PathBuf),
    Remote {
        client: Client,
        url: String,
        digest: Option<String>,
    },
}

pub(super) struct Source {
    pub manifest: Manifest,
    pub manifest_bytes: Vec<u8>,
    pub origin: String,
    location: Location,
}

impl Source {
    /// Resolve an exact SDK version, validate its metadata/checksums, and never use AppCore's latest release.
    pub fn load(version: &str, local: Option<&Path>) -> Result<Self> {
        version_check(version)?;
        let name = format!("x86_64-mochios-sdk-{version}.tar.gz");
        let json_name = format!("{name}.json");
        let (checksums, manifest_bytes, origin, location) = if let Some(directory) = local {
            (
                bounded(File::open(directory.join("SHA256SUMS"))?, MAX_METADATA)?,
                bounded(File::open(directory.join(&json_name))?, MAX_METADATA)?,
                "local".to_string(),
                Location::Local(directory.join(&name)),
            )
        } else {
            let client = Client::builder()
                .user_agent("komeup/0.1.0")
                .https_only(true)
                .connect_timeout(Duration::from_secs(30))
                .timeout(Duration::from_secs(300))
                .redirect(reqwest::redirect::Policy::limited(5))
                .build()?;
            let tag = format!("sdk-{version}");
            let response = client
                .get(format!(
                    "https://api.github.com/repos/mochiOS/devkit/releases/tags/{tag}"
                ))
                .send()?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                bail!("SDK release {tag} is not published. Publish it manually, or install from a local directory with --from");
            }
            let release: Release =
                serde_json::from_slice(&bounded(response.error_for_status()?, MAX_METADATA)?)?;
            if release.draft || release.tag_name != tag {
                bail!("SDK release identity is invalid or still a draft");
            }
            let asset = find(&release, &name)?;
            let checksums = fetch(&client, find(&release, "SHA256SUMS")?, &tag)?;
            let manifest_bytes = fetch(&client, find(&release, &json_name)?, &tag)?;
            check_url(asset, &tag)?;
            (
                checksums,
                manifest_bytes,
                asset.browser_download_url.clone(),
                Location::Remote {
                    client,
                    url: asset.browser_download_url.clone(),
                    digest: asset.digest.clone(),
                },
            )
        };
        let manifest: Manifest =
            serde_json::from_slice(&manifest_bytes).context("Invalid SDK manifest")?;
        let expected = checksum(&checksums, &name)?;
        if manifest.format != 1
            || manifest.archive != name
            || manifest.sha256 != expected
            || !valid_hash(&expected)
        {
            bail!("SDK manifest disagrees with SHA256SUMS");
        }
        Ok(Self {
            manifest,
            manifest_bytes,
            origin,
            location,
        })
    }

    /// Stream an archive to private staging and verify SHA-256 before any extraction.
    pub fn download_archive(&self, destination: &Path) -> Result<()> {
        let mut input: Box<dyn Read> = match &self.location {
            Location::Local(path) => Box::new(File::open(path)?),
            Location::Remote { client, url, .. } => {
                Box::new(client.get(url).send()?.error_for_status()?)
            }
        };
        let mut output = File::create(destination)?;
        let mut digest = Sha256::new();
        let mut total = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let len = input.read(&mut buffer)?;
            if len == 0 {
                break;
            }
            total += len as u64;
            if total > MAX_ARCHIVE {
                bail!("SDK archive exceeds the 2 GiB download limit");
            }
            digest.update(&buffer[..len]);
            output.write_all(&buffer[..len])?;
        }
        output.sync_all()?;
        let actual = format!("{:x}", digest.finalize());
        if actual != self.manifest.sha256 {
            bail!("SDK archive SHA-256 mismatch; installation was not changed");
        }
        if let Location::Remote {
            digest: Some(expected),
            ..
        } = &self.location
        {
            if expected != &format!("sha256:{actual}") {
                bail!("GitHub's SDK asset digest disagrees with SHA256SUMS");
            }
        }
        Ok(())
    }
}

fn bounded(mut input: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    input.by_ref().take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("SDK metadata exceeds its size limit");
    }
    Ok(bytes)
}

fn find<'a>(release: &'a Release, name: &str) -> Result<&'a Asset> {
    let matches: Vec<_> = release
        .assets
        .iter()
        .filter(|asset| asset.name == name)
        .collect();
    if matches.len() != 1 {
        bail!("SDK release must contain exactly one {name}");
    }
    Ok(matches[0])
}

fn check_url(asset: &Asset, tag: &str) -> Result<()> {
    let expected = format!(
        "https://github.com/mochiOS/devkit/releases/download/{tag}/{}",
        asset.name
    );
    if asset.browser_download_url != expected {
        bail!("Unexpected SDK asset URL");
    }
    Ok(())
}

fn fetch(client: &Client, asset: &Asset, tag: &str) -> Result<Vec<u8>> {
    check_url(asset, tag)?;
    let bytes = bounded(
        client
            .get(&asset.browser_download_url)
            .send()?
            .error_for_status()?,
        MAX_METADATA,
    )?;
    if let Some(digest) = &asset.digest {
        if digest != &format!("sha256:{:x}", Sha256::digest(&bytes)) {
            bail!("GitHub asset digest mismatch: {}", asset.name);
        }
    }
    Ok(bytes)
}

fn checksum(bytes: &[u8], expected: &str) -> Result<String> {
    let text = std::str::from_utf8(bytes)?;
    let mut found = None;
    for line in text
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
    {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 2 || !valid_hash(fields[0]) {
            bail!("Malformed SHA256SUMS entry");
        }
        if fields[1] == expected {
            if found.is_some() {
                bail!("Duplicate SDK checksum entry");
            }
            found = Some(fields[0].to_string());
        }
    }
    found.context("SDK archive is not listed in SHA256SUMS")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checksums_require_one_exact_sdk_asset() {
        let name = "x86_64-mochios-sdk-27.0-dp.1-shared.2.tar.gz";
        let line = format!("{}  {name}\n", "a".repeat(64));
        assert_eq!(checksum(line.as_bytes(), name).unwrap(), "a".repeat(64));
        assert!(checksum(format!("{line}{line}").as_bytes(), name).is_err());
        assert!(checksum(line.as_bytes(), "x86_64-appcore-27.0-dp.1.tar.zst").is_err());
        assert!(checksum(b"not-a-hash  filename\n", name).is_err());
    }
    #[test]
    fn asset_urls_cannot_change_repository_or_transport() {
        let name = "SDK.tar.gz";
        let tag = "sdk-fixture";
        for url in [
            "http://github.com/mochiOS/devkit/releases/download/sdk-fixture/SDK.tar.gz",
            "https://example.invalid/SDK.tar.gz",
            "https://github.com/another/repo/releases/download/sdk-fixture/SDK.tar.gz",
        ] {
            assert!(check_url(
                &Asset {
                    name: name.into(),
                    browser_download_url: url.into(),
                    digest: None
                },
                tag
            )
            .is_err());
        }
    }
}
