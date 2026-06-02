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
