use super::*;
use crate::openhuman::security::SecurityPolicy;
use tempfile::TempDir;
fn slash_norm(s: String) -> String {
    s.replace('\\', "/")
}

fn tool(tmp: &TempDir, allow: Vec<&str>) -> CurlTool {
    CurlTool::new(
        Arc::new(SecurityPolicy::default()),
        allow.into_iter().map(String::from).collect(),
        tmp.path().to_path_buf(),
        "downloads".into(),
        1024 * 1024,
        30,
    )
}

#[test]
fn sanitize_dest_subdir_strips_absolute_paths() {
    assert_eq!(
        slash_norm(sanitize_dest_subdir("/etc/passwd")),
        "etc/passwd"
    );
    assert_eq!(sanitize_dest_subdir("//foo"), "foo");
}

#[test]
fn sanitize_dest_subdir_strips_parent_segments() {
    assert_eq!(sanitize_dest_subdir("../../etc"), "etc");
    assert_eq!(slash_norm(sanitize_dest_subdir("a/../b")), "a/b");
}

#[test]
fn sanitize_dest_subdir_falls_back_to_downloads() {
    assert_eq!(sanitize_dest_subdir(""), "downloads");
    assert_eq!(sanitize_dest_subdir("   "), "downloads");
    assert_eq!(sanitize_dest_subdir(".."), "downloads");
    assert_eq!(sanitize_dest_subdir("/"), "downloads");
}

#[test]
fn sanitize_dest_subdir_keeps_normal_paths() {
    assert_eq!(sanitize_dest_subdir("downloads"), "downloads");
    assert_eq!(
        slash_norm(sanitize_dest_subdir("artifacts/build")),
        "artifacts/build"
    );
}

#[test]
fn new_sanitizes_malicious_dest_subdir() {
    let tmp = TempDir::new().unwrap();
    let t = CurlTool::new(
        Arc::new(SecurityPolicy::default()),
        vec!["example.com".into()],
        tmp.path().to_path_buf(),
        "../../etc".into(),
        1024,
        30,
    );
    let resolved = t.resolve_dest("file.txt").unwrap();
    // Sanitizer reduced "../../etc" to "etc"; resolution must stay under workspace.
    assert!(resolved.starts_with(tmp.path().join("etc")));
    assert!(resolved.starts_with(tmp.path()));
}

#[test]
fn resolve_dest_normal() {
    let tmp = TempDir::new().unwrap();
    let t = tool(&tmp, vec!["example.com"]);
    let p = t.resolve_dest("foo/bar.txt").unwrap();
    assert!(p.starts_with(tmp.path().join("downloads")));
    assert!(p.ends_with("foo/bar.txt"));
}

#[test]
fn resolve_dest_rejects_absolute() {
    let tmp = TempDir::new().unwrap();
    let t = tool(&tmp, vec!["example.com"]);
    let err = t.resolve_dest("/etc/passwd").unwrap_err().to_string();
    assert!(err.contains("relative"));
}

#[test]
fn resolve_dest_rejects_parent_dir() {
    let tmp = TempDir::new().unwrap();
    let t = tool(&tmp, vec!["example.com"]);
    let err = t.resolve_dest("../etc/passwd").unwrap_err().to_string();
    assert!(err.contains(".."));
}

#[test]
fn resolve_dest_rejects_nested_parent_dir() {
    let tmp = TempDir::new().unwrap();
    let t = tool(&tmp, vec!["example.com"]);
    let err = t.resolve_dest("a/../../b").unwrap_err().to_string();
    assert!(err.contains(".."));
}

#[test]
fn resolve_dest_rejects_empty() {
    let tmp = TempDir::new().unwrap();
    let t = tool(&tmp, vec!["example.com"]);
    assert!(t.resolve_dest("").is_err());
    assert!(t.resolve_dest("   ").is_err());
}

#[tokio::test]
async fn validate_url_rejects_disallowed_domain() {
    let tmp = TempDir::new().unwrap();
    let t = tool(&tmp, vec!["example.com"]);
    let err = t
        .validate_url("https://evil.test/archive.tar.gz")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("allowed websites"));
}

#[test]
fn default_filename_from_url_basic() {
    assert_eq!(
        CurlTool::default_filename_from_url("https://example.com/foo/bar.zip"),
        "bar.zip"
    );
}

#[test]
fn default_filename_from_url_query_stripped() {
    assert_eq!(
        CurlTool::default_filename_from_url("https://example.com/file.tar.gz?token=x"),
        "file.tar.gz"
    );
}

#[test]
fn default_filename_from_url_root_falls_back() {
    assert_eq!(
        CurlTool::default_filename_from_url("https://example.com/"),
        "download.bin"
    );
}

#[tokio::test]
async fn execute_blocks_when_rate_limited() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy {
        max_actions_per_hour: 0,
        ..SecurityPolicy::default()
    });
    let t = CurlTool::new(
        security,
        vec!["example.com".into()],
        tmp.path().into(),
        "downloads".into(),
        1024,
        30,
    );
    let result = t
        .execute(serde_json::json!({"url": "https://example.com/x"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("rate limit"));
}

#[tokio::test]
async fn execute_blocked_under_local_only_privacy_mode() {
    // Privacy epic S7 (#4441): under LocalOnly the download is refused with a
    // `[policy-blocked]` result before URL validation / network.
    let _mode = super::super::local_only_scope();
    let tmp = TempDir::new().unwrap();
    let t = tool(&tmp, vec!["example.com"]);
    let result = t
        .execute(serde_json::json!({"url": "https://example.com/x"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("[policy-blocked]"),
        "got: {}",
        result.output()
    );
    assert!(
        result.output().contains("Local-only"),
        "got: {}",
        result.output()
    );
}

/// Live integration smoke: downloads example.com (a tiny, stable
/// public page). Gated behind `OPENHUMAN_CURL_LIVE_TEST=1` so CI /
/// offline runs don't depend on the network.
#[tokio::test]
async fn live_download_example_com() {
    if std::env::var("OPENHUMAN_CURL_LIVE_TEST").ok().as_deref() != Some("1") {
        return;
    }
    let tmp = TempDir::new().unwrap();
    let t = tool(&tmp, vec!["example.com"]);
    let result = t
        .execute(serde_json::json!({
            "url": "https://example.com/",
            "dest_path": "example.html"
        }))
        .await
        .unwrap();
    assert!(!result.is_error, "live curl errored: {}", result.output());
    let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
    let bytes = payload["bytes_written"].as_u64().unwrap();
    assert!(bytes > 100, "unexpectedly small download: {bytes} bytes");
    let path = payload["path"].as_str().unwrap();
    let content = std::fs::read_to_string(path).unwrap();
    assert!(content.to_lowercase().contains("example domain"));
}

#[tokio::test]
async fn execute_rejects_allowlist_miss() {
    let tmp = TempDir::new().unwrap();
    let t = tool(&tmp, vec!["example.com"]);
    let result = t
        .execute(serde_json::json!({"url": "https://other.example.org/x"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("allowed websites"));
}

/// The downloads root cannot be created (a plain file already occupies its
/// path) — `create_dir_all` must be reported as a tool error, never a panic,
/// and no network request is attempted before that check runs. The URL uses
/// a public IPv4 literal (never actually contacted — `create_dir_all` fails
/// and returns before any HTTP client is built) so `validate_url_with_dns_check`
/// takes its IP-literal short-circuit and this test needs no live DNS.
#[tokio::test]
async fn execute_reports_a_create_dir_all_failure_rather_than_panicking() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("downloads"), b"not a directory").unwrap();
    let t = tool(&tmp, vec!["8.8.8.8"]);
    let result = t
        .execute(serde_json::json!({"url": "https://8.8.8.8/x", "dest_path": "sub/file.txt"}))
        .await
        .unwrap();
    assert!(
        result.is_error,
        "an unwritable downloads root must be reported, not panic: {}",
        result.output()
    );
    assert!(
        result
            .output()
            .contains("Failed to create destination directory"),
        "must fail at the create_dir_all step specifically, not e.g. DNS resolution: {}",
        result.output()
    );
}
