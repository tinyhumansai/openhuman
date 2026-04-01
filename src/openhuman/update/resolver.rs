use serde::Deserialize;

use super::types::UpdateAsset;

const DEFAULT_UPDATE_REPO: &str = "tinyhumansai/openhuman";
const DEFAULT_UPDATE_API_BASE: &str = "https://api.github.com";

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: Option<String>,
    html_url: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    #[serde(default)]
    browser_download_url: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    digest: Option<String>,
}

fn normalize_version(value: &str) -> Option<String> {
    let base = value.trim().trim_start_matches('v');
    if base.is_empty() {
        return None;
    }
    let core = base.split('-').next().unwrap_or(base);
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(format!("{major}.{minor}.{patch}"))
}

pub fn compare_versions(a: &str, b: &str) -> Result<std::cmp::Ordering, String> {
    fn parse(v: &str) -> Result<(u64, u64, u64), String> {
        let n = normalize_version(v).ok_or_else(|| format!("invalid version: {v}"))?;
        let mut parts = n.split('.');
        let major = parts
            .next()
            .ok_or_else(|| format!("invalid version: {v}"))?
            .parse::<u64>()
            .map_err(|_| format!("invalid version: {v}"))?;
        let minor = parts
            .next()
            .ok_or_else(|| format!("invalid version: {v}"))?
            .parse::<u64>()
            .map_err(|_| format!("invalid version: {v}"))?;
        let patch = parts
            .next()
            .ok_or_else(|| format!("invalid version: {v}"))?
            .parse::<u64>()
            .map_err(|_| format!("invalid version: {v}"))?;
        Ok((major, minor, patch))
    }

    Ok(parse(a)?.cmp(&parse(b)?))
}

fn target_triple() -> String {
    std::env::var("OPENHUMAN_UPDATE_TARGET")
        .or_else(|_| std::env::var("TARGET"))
        .unwrap_or_else(|_| {
            #[cfg(target_os = "windows")]
            let os = "pc-windows-msvc";
            #[cfg(target_os = "macos")]
            let os = "apple-darwin";
            #[cfg(target_os = "linux")]
            let os = "unknown-linux-gnu";
            #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
            let os = std::env::consts::OS;

            let arch = match std::env::consts::ARCH {
                "x86_64" => "x86_64",
                "aarch64" => "aarch64",
                "arm64" => "aarch64",
                other => other,
            };
            format!("{arch}-{os}")
        })
}

fn expected_asset_name(version: &str) -> String {
    let target = target_triple();
    let ext = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    format!("openhuman-core_{version}_{target}{ext}")
}

fn digest_without_prefix(value: &str) -> String {
    value
        .trim()
        .strip_prefix("sha256:")
        .unwrap_or(value.trim())
        .to_ascii_lowercase()
}

pub struct ResolvedRelease {
    pub latest: Option<UpdateAsset>,
    pub etag: Option<String>,
    pub not_modified: bool,
}

pub async fn fetch_latest_release(last_etag: Option<&str>) -> Result<ResolvedRelease, String> {
    let repo =
        std::env::var("OPENHUMAN_UPDATE_REPO").unwrap_or_else(|_| DEFAULT_UPDATE_REPO.into());
    let api_base = std::env::var("OPENHUMAN_UPDATE_API_BASE")
        .unwrap_or_else(|_| DEFAULT_UPDATE_API_BASE.into())
        .trim_end_matches('/')
        .to_string();
    let url = format!("{api_base}/repos/{repo}/releases/latest");
    log::debug!("[update] fetching latest release from {url} (etag={last_etag:?})");

    let client = crate::openhuman::config::build_runtime_proxy_client_with_timeouts(
        "update.release-check",
        30, // 30-second total timeout for API call
        10, // 10-second connect timeout
    );

    let mut req = client
        .get(url)
        .header("User-Agent", "openhuman-core-updater")
        .header("Accept", "application/vnd.github+json");

    if let Some(etag) = last_etag.filter(|v| !v.trim().is_empty()) {
        req = req.header("If-None-Match", etag);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("release check request failed: {e}"))?;

    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(ResolvedRelease {
            latest: None,
            etag: last_etag.map(ToOwned::to_owned),
            not_modified: true,
        });
    }

    if !resp.status().is_success() {
        return Err(format!(
            "release check failed with status {}",
            resp.status().as_u16()
        ));
    }

    let etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let release: GitHubRelease = resp
        .json()
        .await
        .map_err(|e| format!("failed to decode latest release payload: {e}"))?;

    let tag = release
        .tag_name
        .clone()
        .ok_or_else(|| "latest release missing tag_name".to_string())?;
    let version = normalize_version(&tag)
        .ok_or_else(|| format!("latest release tag is not semver-like: {tag}"))?;
    let expected_name = expected_asset_name(&version);

    log::debug!(
        "[update] resolved release tag={tag} version={version}, looking for asset={expected_name}"
    );

    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name == expected_name)
        .ok_or_else(|| {
            format!(
                "no compatible core asset in latest release for target {} (expected: {expected_name})",
                target_triple()
            )
        })?;

    let download_url = asset
        .browser_download_url
        .or(asset.url)
        .ok_or_else(|| format!("asset '{expected_name}' missing download URL"))?;

    let digest_sha256 = asset
        .digest
        .as_deref()
        .map(digest_without_prefix)
        .filter(|v| !v.is_empty());

    Ok(ResolvedRelease {
        latest: Some(UpdateAsset {
            version,
            tag,
            name: asset.name,
            download_url,
            digest_sha256,
            release_url: release.html_url.unwrap_or_default(),
        }),
        etag,
        not_modified: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_and_compare_versions() {
        assert_eq!(normalize_version("v1.2.3").as_deref(), Some("1.2.3"));
        assert_eq!(normalize_version("1.2.3-beta.1").as_deref(), Some("1.2.3"));
        assert_eq!(
            compare_versions("v1.2.3", "1.2.2").expect("version compare should parse"),
            std::cmp::Ordering::Greater
        );
        assert_eq!(
            compare_versions("1.2.3", "1.2.3").expect("version compare should parse"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn expected_asset_name_contains_target() {
        let name = expected_asset_name("0.49.32");
        assert!(name.contains("openhuman-core_0.49.32_"));
    }
}
