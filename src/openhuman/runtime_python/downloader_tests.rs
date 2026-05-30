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

#[test]
fn rejects_prerelease_versions() {
    // python-build-standalone publishes betas alongside stable patch
    // releases (`3.15.0b1+20260510-...`). parse_python_version is
    // lenient enough to parse the version as 3.15.0, so without the
    // pre-release guard the selector would happily pick a beta whose
    // third-party wheel ecosystem is bare (Pillow et al), and pip's
    // source-build fallback crashes on missing toolchain.
    let beta = GithubAsset {
        name: "cpython-3.15.0b1+20260510-aarch64-apple-darwin-install_only_stripped.tar.gz"
            .to_string(),
        browser_download_url: "https://example.invalid/python-beta.tar.gz".to_string(),
        digest: Some("sha256:def456".to_string()),
    };
    assert!(parse_distribution_asset(&beta, "20260510").is_none());

    let alpha = GithubAsset {
        name: "cpython-3.16.0a2+20260510-aarch64-apple-darwin-install_only.tar.gz".to_string(),
        browser_download_url: "https://example.invalid/python-alpha.tar.gz".to_string(),
        digest: None,
    };
    assert!(parse_distribution_asset(&alpha, "20260510").is_none());

    let rc = GithubAsset {
        name: "cpython-3.14.0rc1+20260510-x86_64-unknown-linux-gnu-install_only.tar.gz".to_string(),
        browser_download_url: "https://example.invalid/python-rc.tar.gz".to_string(),
        digest: None,
    };
    assert!(parse_distribution_asset(&rc, "20260510").is_none());
}
