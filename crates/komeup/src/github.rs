use anyhow::{Context, Result};
use reqwest::blocking::Client;
use serde::Deserialize;

const OWNER: &str = "mochiOS";

#[derive(Debug, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    pub assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
}

pub fn client() -> Result<Client> {
    let client = Client::builder()
        .user_agent("komeup/0.1.0")
        .build()
        .context("failed to create HTTP client")?;

    Ok(client)
}

pub fn latest_release(client: &Client, repo: &str) -> Result<GithubRelease> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        OWNER, repo
    );

    let release = client
        .get(&url)
        .send()
        .with_context(|| format!("failed to request {}", url))?
        .error_for_status()
        .with_context(|| format!("GitHub API returned an error for {}", url))?
        .json::<GithubRelease>()
        .with_context(|| format!("failed to parse release response for {}", repo))?;

    Ok(release)
}

pub fn find_asset_exact(release: &GithubRelease, name: &str) -> Result<GithubAsset> {
    release
        .assets
        .iter()
        .find(|asset| asset.name == name)
        .cloned()
        .with_context(|| {
            format!(
                "asset '{}' was not found in release {}",
                name,
                release.tag_name
            )
        })
}

pub fn find_std_asset(release: &GithubRelease) -> Result<GithubAsset> {
    let expected_with_v = format!("{}.tar.gz", release.tag_name);
    let expected_without_v = format!("{}.tar.gz", release.tag_name.trim_start_matches('v'));

    if let Some(asset) = release.assets.iter().find(|asset| asset.name == expected_with_v) {
        return Ok(asset.clone());
    }

    if let Some(asset) = release.assets.iter().find(|asset| asset.name == expected_without_v) {
        return Ok(asset.clone());
    }

    release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(".tar.gz"))
        .cloned()
        .with_context(|| {
            format!(
                "std archive was not found in release {}",
                release.tag_name
            )
        })
}

pub fn download_bytes(client: &Client, url: &str) -> Result<Vec<u8>> {
    let bytes = client
        .get(url)
        .send()
        .with_context(|| format!("failed to download {}", url))?
        .error_for_status()
        .with_context(|| format!("download failed: {}", url))?
        .bytes()
        .with_context(|| format!("failed to read response body: {}", url))?;

    Ok(bytes.to_vec())
}