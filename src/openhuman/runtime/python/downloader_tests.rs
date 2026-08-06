use super::*;

#[test]
fn parses_asset_into_distribution() {
    let asset = GithubAsset {
        name: "cpython-3.12.13+20260510-x86_64-apple-darwin-install_only.tar.gz".to_string(),
        browser_download_url: "https://example.invalid/python.tar.gz".to_string(),
        digest: Some("sha256:abc123".to_string()),
    };
    let dist = parse_distribution_asset(&asset, "20260510").expect("dist");
    assert_eq!(dist.release_tag, "20260510");
    assert_eq!(dist.version.display(), "3.12.13");
    assert_eq!(dist.expected_sha256.as_deref(), Some("abc123"));
}

#[test]
fn ignores_non_install_only_assets() {
    let asset = GithubAsset {
        name: "cpython-3.12.13+20260510-x86_64-apple-darwin-full.tar.zst".to_string(),
        browser_download_url: "https://example.invalid/python.tar.zst".to_string(),
        digest: None,
    };
    assert!(parse_distribution_asset(&asset, "20260510").is_none());
}

/// Build a release with one `install_only` asset per supplied version, named
/// for the current host so `select_distribution` accepts them.
fn release_with_versions(versions: &[&str]) -> GithubRelease {
    let suffix = host_asset_suffix().expect("host suffix");
    let assets = versions
        .iter()
        .map(|v| GithubAsset {
            name: format!("cpython-{v}+20260623-{suffix}"),
            browser_download_url: format!("https://example.invalid/{v}.tar.gz"),
            digest: None,
        })
        .collect();
    GithubRelease {
        tag_name: "20260623".to_string(),
        assets,
    }
}

#[test]
fn maximum_version_caps_selection_to_stable_series() {
    // 3.15.0b3 parses to a bare 3.15.0 — the cap is what keeps us off it.
    let release = release_with_versions(&["3.12.13", "3.13.5", "3.15.0"]);
    let dist = select_distribution(&release, "3.12.0", "3.14.0").expect("dist");
    assert_eq!(dist.version.display(), "3.13.5");
}

#[test]
fn empty_maximum_version_disables_the_cap() {
    let release = release_with_versions(&["3.13.5", "3.15.0"]);
    let dist = select_distribution(&release, "3.12.0", "").expect("dist");
    assert_eq!(dist.version.display(), "3.15.0");
}

#[test]
fn invalid_maximum_version_is_an_error() {
    let release = release_with_versions(&["3.13.5"]);
    assert!(select_distribution(&release, "3.12.0", "not-a-version").is_err());
}
