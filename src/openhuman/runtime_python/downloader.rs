//! Managed standalone Python distribution downloader.
//!
//! Pulls release metadata from `astral-sh/python-build-standalone`, selects a
//! host-compatible `install_only` archive satisfying the configured minimum
//! Python version, downloads it, and verifies the published SHA-256 digest.

use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use super::resolver::{parse_python_version, PythonVersion};

pub(crate) const RELEASES_API: &str =
    "https://api.github.com/repos/astral-sh/python-build-standalone/releases";

#[derive(Debug, Clone, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    pub assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub digest: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PythonDistribution {
    pub release_tag: String,
    pub asset_name: String,
    pub url: String,
    pub version: PythonVersion,
    pub expected_sha256: Option<String>,
}

impl PythonDistribution {
    pub fn install_dir_name(&self) -> String {
        self.asset_name.trim_end_matches(".tar.gz").to_string()
    }
}

pub async fn fetch_release_metadata(
    client: &Client,
    release_tag: Option<&str>,
) -> Result<GithubRelease> {
    fetch_release_metadata_from_base(client, RELEASES_API, release_tag).await
}

pub(crate) async fn fetch_release_metadata_from_base(
    client: &Client,
    releases_api_base: &str,
    release_tag: Option<&str>,
) -> Result<GithubRelease> {
    let url = if let Some(tag) = release_tag {
        format!("{releases_api_base}/tags/{tag}")
    } else {
        format!("{releases_api_base}/latest")
    };

    tracing::debug!(url = %url, "[runtime_python::downloader] fetching release metadata");

    client
        .get(&url)
        .header(reqwest::header::USER_AGENT, "openhuman-core/runtime_python")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("non-success status on {url}"))?
        .json::<GithubRelease>()
        .await
        .with_context(|| format!("decoding release metadata from {url}"))
}

pub fn select_distribution(
    release: &GithubRelease,
    minimum_version: &str,
) -> Result<PythonDistribution> {
    let Some(minimum) = parse_python_version(minimum_version) else {
        bail!("invalid runtime_python.minimum_version `{minimum_version}`");
    };
    let target_suffix = host_asset_suffix()?;

    let mut candidates = release
        .assets
        .iter()
        .filter_map(|asset| parse_distribution_asset(asset, &release.tag_name))
        .filter(|dist| asset_matches_target(&dist.asset_name, target_suffix))
        .filter(|dist| dist.version >= minimum)
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        bail!(
            "no managed python-build-standalone asset found for host suffix `{target_suffix}` with version >= {} in release {}",
            minimum.display(),
            release.tag_name
        );
    }

    candidates.sort_by(|a, b| {
        b.version
            .cmp(&a.version)
            .then_with(|| a.asset_name.cmp(&b.asset_name))
    });

    if let Some(preferred) = candidates
        .iter()
        .find(|dist| dist.asset_name.contains("install_only_stripped"))
        .cloned()
    {
        return Ok(preferred);
    }

    candidates
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("internal error selecting managed python asset"))
}

fn parse_distribution_asset(asset: &GithubAsset, release_tag: &str) -> Option<PythonDistribution> {
    let name = asset.name.as_str();
    if !name.starts_with("cpython-") || !name.ends_with(".tar.gz") || !name.contains("install_only")
    {
        return None;
    }

    let rest = name.strip_prefix("cpython-")?;
    let version_str = rest.split('+').next()?;
    let version = parse_python_version(version_str)?;

    let expected_sha256 = asset
        .digest
        .as_deref()
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .map(str::to_string);

    Some(PythonDistribution {
        release_tag: release_tag.to_string(),
        asset_name: asset.name.clone(),
        url: asset.browser_download_url.clone(),
        version,
        expected_sha256,
    })
}

fn host_asset_suffix() -> Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin-install_only.tar.gz"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin-install_only.tar.gz"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu-install_only.tar.gz"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu-install_only.tar.gz"),
        ("windows", "aarch64") => Ok("aarch64-pc-windows-msvc-install_only.tar.gz"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc-install_only.tar.gz"),
        _ => Err(anyhow!(
            "no managed standalone Python distribution for host {os}/{arch}"
        )),
    }
}

fn asset_matches_target(asset_name: &str, target_suffix: &str) -> bool {
    asset_name.ends_with(target_suffix)
        || asset_name.ends_with(
            &target_suffix.replace("-install_only.tar.gz", "-install_only_stripped.tar.gz"),
        )
}

pub async fn download_distribution(
    client: &Client,
    dist: &PythonDistribution,
    target_path: &Path,
) -> Result<()> {
    tracing::info!(
        url = %dist.url,
        target = %target_path.display(),
        "[runtime_python::downloader] starting download"
    );

    if let Some(parent) = target_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating cache dir {}", parent.display()))?;
    }

    let mut response = client
        .get(&dist.url)
        .header(reqwest::header::USER_AGENT, "openhuman-core/runtime_python")
        .send()
        .await
        .with_context(|| format!("GET {}", dist.url))?
        .error_for_status()
        .with_context(|| format!("non-success status on {}", dist.url))?;

    let mut file = File::create(target_path)
        .await
        .with_context(|| format!("creating {}", target_path.display()))?;
    let mut hasher = Sha256::new();

    let stream_result: Result<()> = async {
        while let Some(chunk) = response
            .chunk()
            .await
            .with_context(|| format!("streaming {}", dist.url))?
        {
            hasher.update(&chunk);
            file.write_all(&chunk)
                .await
                .with_context(|| format!("writing chunk to {}", target_path.display()))?;
        }
        file.flush()
            .await
            .with_context(|| format!("flushing {}", target_path.display()))?;
        Ok(())
    }
    .await;

    drop(file);

    if let Err(err) = stream_result {
        let _ = tokio::fs::remove_file(target_path).await;
        return Err(err);
    }

    if let Some(expected) = dist.expected_sha256.as_deref() {
        let actual_hex = hex::encode(hasher.finalize());
        if actual_hex != expected {
            let _ = tokio::fs::remove_file(target_path).await;
            bail!(
                "SHA-256 mismatch for {} (expected {expected}, got {actual_hex})",
                dist.asset_name
            );
        }
    } else {
        tracing::warn!(
            asset = %dist.asset_name,
            "[runtime_python::downloader] release metadata did not include a digest; skipping SHA-256 verification"
        );
    }

    tracing::info!(
        target = %target_path.display(),
        asset = %dist.asset_name,
        "[runtime_python::downloader] download complete"
    );
    Ok(())
}

#[cfg(test)]
#[path = "downloader_tests.rs"]
mod tests;
