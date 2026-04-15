//! JSON-RPC / CLI controller surface for the update domain.

use std::path::PathBuf;

use serde_json::Value;

use crate::openhuman::update;
use crate::rpc::RpcOutcome;

/// Check GitHub Releases for a newer version of the core binary.
pub async fn update_check() -> RpcOutcome<Value> {
    log::info!("[update:rpc] update_check invoked");
    match update::check_available().await {
        Ok(info) => {
            let value = serde_json::to_value(&info).unwrap_or_else(
                |e| serde_json::json!({ "error": format!("serialization failed: {e}") }),
            );
            RpcOutcome::single_log(value, "update_check completed")
        }
        Err(e) => {
            log::error!("[update:rpc] update_check failed: {e}");
            RpcOutcome::single_log(
                serde_json::json!({ "error": e }),
                format!("update_check failed: {e}"),
            )
        }
    }
}

/// Validate that a download URL points to a GitHub release asset.
fn validate_download_url(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid download URL: {e}"))?;

    let host = parsed.host_str().unwrap_or("");
    if host != "github.com" && host != "api.github.com" && !host.ends_with(".githubusercontent.com")
    {
        return Err(format!(
            "download URL must be a GitHub domain, got '{host}'"
        ));
    }

    if parsed.scheme() != "https" {
        return Err("download URL must use HTTPS".to_string());
    }

    Ok(())
}

/// Validate asset_name is a safe filename (no path separators or traversal).
fn validate_asset_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("asset_name must not be empty".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(format!(
            "asset_name must not contain path separators or '..', got '{name}'"
        ));
    }
    if !name.starts_with("openhuman-core-") {
        return Err(format!(
            "asset_name must start with 'openhuman-core-', got '{name}'"
        ));
    }
    Ok(())
}

/// Download and stage the updated binary to a given path.
///
/// Params:
///   - `download_url` (string, required): must be a GitHub release asset URL (HTTPS).
///   - `asset_name` (string, required): must be a safe filename starting with `openhuman-core-`.
///   - `staging_dir` (string, optional): ignored — always uses the default staging directory
///     for security (next to the running executable or Resources/).
pub async fn update_apply(
    download_url: String,
    asset_name: String,
    _staging_dir: Option<String>,
) -> RpcOutcome<Value> {
    log::info!(
        "[update:rpc] update_apply invoked — url={} asset={}",
        download_url,
        asset_name,
    );

    // Validate inputs at the RPC boundary.
    if let Err(e) = validate_download_url(&download_url) {
        log::error!("[update:rpc] rejected download URL: {e}");
        return RpcOutcome::single_log(
            serde_json::json!({ "error": e }),
            format!("update_apply rejected: {e}"),
        );
    }
    if let Err(e) = validate_asset_name(&asset_name) {
        log::error!("[update:rpc] rejected asset name: {e}");
        return RpcOutcome::single_log(
            serde_json::json!({ "error": e }),
            format!("update_apply rejected: {e}"),
        );
    }

    // Ignore caller-provided staging_dir — always use the safe default.
    let dir: Option<PathBuf> = None;
    match update::download_and_stage(&download_url, &asset_name, dir).await {
        Ok(result) => {
            let value = serde_json::to_value(&result).unwrap_or_else(
                |e| serde_json::json!({ "error": format!("serialization failed: {e}") }),
            );
            RpcOutcome::single_log(value, "update_apply completed")
        }
        Err(e) => {
            log::error!("[update:rpc] update_apply failed: {e}");
            RpcOutcome::single_log(
                serde_json::json!({ "error": e }),
                format!("update_apply failed: {e}"),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── validate_download_url ─────────────────────────────────────

    #[test]
    fn validate_download_url_accepts_github_https_hosts() {
        for url in [
            "https://github.com/owner/repo/releases/download/v1/asset.tar.gz",
            "https://api.github.com/repos/owner/repo/releases/assets/1",
            "https://objects.githubusercontent.com/release-asset/123",
        ] {
            validate_download_url(url).unwrap_or_else(|e| panic!("`{url}` rejected: {e}"));
        }
    }

    #[test]
    fn validate_download_url_rejects_non_github_hosts() {
        let err = validate_download_url("https://evil.example.com/asset.tar.gz").unwrap_err();
        assert!(err.contains("must be a GitHub domain"), "got: {err}");
    }

    #[test]
    fn validate_download_url_rejects_non_https_schemes() {
        let err =
            validate_download_url("http://github.com/owner/repo/releases/download/v1/x").unwrap_err();
        assert!(err.contains("must use HTTPS"), "got: {err}");
    }

    #[test]
    fn validate_download_url_rejects_malformed_url() {
        let err = validate_download_url("not a url").unwrap_err();
        assert!(err.contains("invalid download URL"), "got: {err}");
    }

    // ── validate_asset_name ───────────────────────────────────────

    #[test]
    fn validate_asset_name_accepts_well_formed_core_asset() {
        validate_asset_name("openhuman-core-aarch64-apple-darwin.tar.gz")
            .expect("canonical asset name should be accepted");
    }

    #[test]
    fn validate_asset_name_rejects_empty_string() {
        let err = validate_asset_name("").unwrap_err();
        assert!(err.contains("must not be empty"));
    }

    #[test]
    fn validate_asset_name_rejects_path_separators_and_traversal() {
        for bad in [
            "openhuman-core-../etc/passwd",
            "../openhuman-core-x86.tar.gz",
            "openhuman-core/x86.tar.gz",
            "openhuman-core\\x86.tar.gz",
        ] {
            let err = validate_asset_name(bad).unwrap_err();
            assert!(
                err.contains("path separators") || err.contains("'..'"),
                "input `{bad}` produced unexpected error: {err}"
            );
        }
    }

    #[test]
    fn validate_asset_name_rejects_unprefixed_asset() {
        let err = validate_asset_name("malicious-binary.tar.gz").unwrap_err();
        assert!(
            err.contains("must start with 'openhuman-core-'"),
            "got: {err}"
        );
    }

    // ── update_apply rejection paths ──────────────────────────────

    #[tokio::test]
    async fn update_apply_rejects_non_github_url_before_network_call() {
        let outcome = update_apply(
            "https://evil.example.com/asset".to_string(),
            "openhuman-core-x86_64.tar.gz".to_string(),
            None,
        )
        .await;
        assert!(outcome.value.get("error").is_some());
        assert!(outcome
            .logs
            .iter()
            .any(|l| l.contains("update_apply rejected")));
    }

    #[tokio::test]
    async fn update_apply_rejects_unsafe_asset_name() {
        let outcome = update_apply(
            "https://github.com/owner/repo/releases/download/v1/x".to_string(),
            "../etc/passwd".to_string(),
            None,
        )
        .await;
        assert!(outcome.value.get("error").is_some());
        assert!(outcome
            .logs
            .iter()
            .any(|l| l.contains("update_apply rejected")));
    }

    // NOTE: `update_check` and the success path of `update_apply`
    // hit GitHub's REST API and stage real binaries on disk — they
    // are deferred to the integration test suite (tests/) where a
    // real network fixture or recorded cassette is available.
}
