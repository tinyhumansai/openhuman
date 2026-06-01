use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory_sources::readers::SourceReader;
use openhuman_core::openhuman::memory_sources::{ContentType, MemorySourceEntry, SourceKind};
use tempfile::{Builder, TempDir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set_path(key: &'static str, value: &Path) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value.as_os_str()) };
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn tempdir() -> TempDir {
    std::fs::create_dir_all("target").expect("target dir");
    Builder::new()
        .prefix("memory-sources-readers-round21-")
        .tempdir_in("target")
        .expect("tempdir")
}

fn config(tmp: &TempDir) -> Config {
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.action_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config
}

fn source_entry(id: &str, kind: SourceKind) -> MemorySourceEntry {
    MemorySourceEntry {
        id: id.to_string(),
        kind,
        label: format!("{id} label"),
        enabled: true,
        toolkit: None,
        connection_id: None,
        path: None,
        glob: None,
        url: None,
        branch: None,
        paths: Vec::new(),
        query: None,
        since_days: None,
        max_items: None,
        max_commits: None,
        max_issues: None,
        max_prs: None,
        selector: None,
    }
}

async fn one_response_server(
    status: &'static str,
    headers: &'static str,
    body: Vec<u8>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind fixture");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let task = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut req = [0_u8; 1024];
            let _ = stream.read(&mut req).await;
            let has_content_length = headers
                .lines()
                .any(|line| line.to_ascii_lowercase().starts_with("content-length:"));
            let mut response = format!("HTTP/1.1 {status}\r\n");
            if !has_content_length {
                response.push_str(&format!("content-length: {}\r\n", body.len()));
            }
            if !headers.is_empty() {
                response.push_str(headers);
                response.push_str("\r\n");
            }
            response.push_str("\r\n");
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.write_all(&body).await;
            let _ = stream.shutdown().await;
        }
    });
    (url, task)
}

#[tokio::test]
async fn round21_rss_reader_covers_http_body_guards_and_invalid_utf8() {
    let _lock = env_lock();
    let tmp = tempdir();
    let config = config(&tmp);
    let reader = openhuman_core::openhuman::memory_sources::readers::rss::RssReader;

    let (status_url, status_server) =
        one_response_server("503 Service Unavailable", "", b"down".to_vec()).await;
    let status_err = reader
        .list_items(
            &MemorySourceEntry {
                url: Some(status_url),
                ..source_entry("rss-status", SourceKind::RssFeed)
            },
            &config,
        )
        .await
        .expect_err("non-success status rejected");
    assert!(status_err.contains("feed returned 503"));
    let _ = status_server.await;

    let huge_len = 5 * 1024 * 1024 + 1;
    let (large_url, large_server) = one_response_server(
        "200 OK",
        "content-type: application/rss+xml\r\ncontent-length: 5242881",
        b"<rss/>".to_vec(),
    )
    .await;
    let large_err = reader
        .list_items(
            &MemorySourceEntry {
                url: Some(large_url),
                ..source_entry("rss-large", SourceKind::RssFeed)
            },
            &config,
        )
        .await
        .expect_err("large feed rejected from content-length");
    assert!(large_err.contains(&format!("feed body too large: {huge_len} bytes")));
    let _ = large_server.await;

    let (utf8_url, utf8_server) = one_response_server(
        "200 OK",
        "content-type: application/rss+xml",
        vec![0xff, 0xfe],
    )
    .await;
    let utf8_err = reader
        .list_items(
            &MemorySourceEntry {
                url: Some(utf8_url),
                ..source_entry("rss-utf8", SourceKind::RssFeed)
            },
            &config,
        )
        .await
        .expect_err("invalid utf8 feed rejected");
    assert!(utf8_err.contains("feed body is not valid UTF-8"));
    let _ = utf8_server.await;
}

#[tokio::test]
async fn round21_github_reader_covers_commit_issue_comments_and_error_paths() {
    let _lock = env_lock();
    // This test requires a fake `gh` on PATH. If the real `gh` is not
    // installed (CI containers), gh_available() returns false and the reader
    // falls through to the real GitHub API which rate-limits. Skip gracefully.
    if std::process::Command::new("gh")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| !s.success())
        .unwrap_or(true)
    {
        eprintln!("skipping: gh CLI not available");
        return;
    }
    let tmp = tempdir();
    let config = config(&tmp);
    let bin = tmp.path().join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");
    let script = bin.join("gh");
    write_fake_gh(&script);
    let old_path = std::env::var("PATH").unwrap_or_default();
    let _path = EnvGuard::set_path("PATH", Path::new(&format!("{}:{old_path}", bin.display())));

    let reader = openhuman_core::openhuman::memory_sources::readers::github::GithubReader;
    let entry = MemorySourceEntry {
        url: Some("https://github.com/tinyhumansai/openhuman".to_string()),
        ..source_entry("github-round21", SourceKind::GithubRepo)
    };

    let items = reader
        .list_items(&entry, &config)
        .await
        .expect("list items");
    assert!(items.iter().any(|item| item.id == "commit:abc123"));
    assert!(items.iter().any(|item| item.id == "issue:42"));
    assert!(items.iter().any(|item| item.id == "pr:43"));

    let commit = reader
        .read_item(&entry, "commit:abc123", &config)
        .await
        .expect("read commit");
    assert_eq!(commit.content_type, ContentType::Markdown);
    assert!(commit.body.contains("Round21 commit subject"));

    let issue = reader
        .read_item(&entry, "issue:42", &config)
        .await
        .expect("read issue");
    assert!(issue.body.contains("## Description"));
    assert!(issue.body.contains("Issue body"));
    assert_eq!(
        issue
            .metadata
            .get("labels")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(0)
    );

    let bad_pr = reader
        .read_item(&entry, "pr:not-a-number", &config)
        .await
        .expect_err("bad pr number rejected");
    assert!(bad_pr.contains("invalid PR number"));
}

fn write_fake_gh(path: &PathBuf) {
    let script = r#"#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then
  echo "gh version 2.0.0"
  exit 0
fi
if [[ "${1:-}" != "api" ]]; then
  echo "unsupported gh command" >&2
  exit 2
fi
arg="${2:-}"
# Match on the base resource path, ignoring per_page/page/state params.
case "$arg" in
  repos/tinyhumansai/openhuman/commits\?*)
    cat <<'JSON'
[{"sha":"abc123","commit":{"message":"Round21 commit subject\n\nBody line","author":{"name":"Ada","email":"ada@example.test","date":"2026-05-30T00:00:00Z"},"committer":{"name":"Ada","email":"ada@example.test","date":"2026-05-30T00:00:00Z"}}}]
JSON
    ;;
  repos/tinyhumansai/openhuman/issues\?*)
    cat <<'JSON'
[{"number":42,"title":"Round21 issue","body":"Issue body","state":"open","user":{"login":"octo"},"labels":[],"created_at":"2026-05-30T00:00:00Z","updated_at":"2026-05-30T00:01:00Z","pull_request":null}]
JSON
    ;;
  repos/tinyhumansai/openhuman/pulls\?*)
    cat <<'JSON'
[{"number":43,"title":"Round21 PR","body":"PR body","state":"open","user":{"login":"octo"},"labels":[],"created_at":"2026-05-30T00:00:00Z","updated_at":"2026-05-30T00:02:00Z","merged_at":null,"comments":1}]
JSON
    ;;
  repos/tinyhumansai/openhuman/commits/abc123)
    cat <<'JSON'
{"sha":"abc123","commit":{"message":"Round21 commit subject\n\nBody line","author":{"name":"Ada","email":"ada@example.test","date":"2026-05-30T00:00:00Z"},"committer":{"name":"Grace","email":"grace@example.test","date":"2026-05-30T00:03:00Z"}}}
JSON
    ;;
  repos/tinyhumansai/openhuman/issues/42)
    cat <<'JSON'
{"number":42,"title":"Round21 issue","body":"Issue body","state":"open","user":{"login":"octo"},"labels":[],"created_at":"2026-05-30T00:00:00Z","updated_at":"2026-05-30T00:01:00Z","pull_request":null}
JSON
    ;;
  repos/tinyhumansai/openhuman/issues/42/comments\?*)
    cat <<'JSON'
[{"user":{"login":"reviewer"},"body":"Looks good from the fixture","created_at":"2026-05-30T00:04:00Z"}]
JSON
    ;;
  *)
    echo "unexpected gh api path: $arg (stripped: $stripped)" >&2
    exit 3
    ;;
esac
"#;
    std::fs::write(path, script).expect("write fake gh");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod fake gh");
    }
}
