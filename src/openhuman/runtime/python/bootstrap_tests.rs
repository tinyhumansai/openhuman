use super::*;
use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::net::TcpListener;

#[cfg(unix)]
#[tokio::test]
async fn install_managed_from_mock_astral_release_downloads_and_resolves_executable() {
    use std::os::unix::fs::PermissionsExt;

    let archive_bytes = build_test_python_archive().expect("archive bytes");
    let archive_sha = hex::encode(Sha256::digest(&archive_bytes));
    let asset_name = test_asset_name();

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");

    let app_state = Arc::new(MockAstralState {
        asset_name: asset_name.to_string(),
        archive_bytes,
        archive_sha,
        artifact_url: format!("http://{addr}/artifacts/{asset_name}"),
    });

    let app = Router::new()
        .route("/releases/latest", get(mock_release_latest))
        .route("/artifacts/{asset}", get(mock_artifact))
        .with_state(app_state.clone());

    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let tmp = tempfile::tempdir().expect("tempdir");
    let mut cfg = RuntimePythonConfig::default();
    cfg.cache_dir = tmp.path().join("runtime-python").display().to_string();
    cfg.managed_release_tag = String::new();
    cfg.prefer_system = false;

    let bootstrap = PythonBootstrap::new_with_client(cfg, reqwest::Client::new());
    let api_base = format!("http://{addr}/releases");

    let resolved = bootstrap
        .install_managed_from_api(&api_base)
        .await
        .expect("managed install should succeed");

    assert_eq!(resolved.source, PythonSource::Managed);
    assert!(resolved.python_bin.is_file(), "python binary should exist");

    let mode = std::fs::metadata(&resolved.python_bin)
        .expect("metadata")
        .permissions()
        .mode();
    assert_ne!(mode & 0o111, 0, "python binary must be executable");

    let version = crate::openhuman::runtime::python::resolver::probe_python_version_public(
        &resolved.python_bin,
    )
    .expect("version probe");
    assert_eq!(version.trim(), "Python 3.12.13");

    server.abort();
}

#[derive(Clone)]
struct MockAstralState {
    asset_name: String,
    archive_bytes: Vec<u8>,
    archive_sha: String,
    artifact_url: String,
}

async fn mock_release_latest(State(state): State<Arc<MockAstralState>>) -> impl IntoResponse {
    Json(json!({
        "tag_name": "20260510",
        "assets": [
            {
                "name": state.asset_name,
                "browser_download_url": state.artifact_url,
                "digest": format!("sha256:{}", state.archive_sha),
            }
        ]
    }))
}

async fn mock_artifact(
    State(state): State<Arc<MockAstralState>>,
    axum::extract::Path(asset): axum::extract::Path<String>,
) -> impl IntoResponse {
    if asset != state.asset_name {
        return (StatusCode::NOT_FOUND, Vec::new()).into_response();
    }

    (
        [(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/gzip"),
        )],
        state.archive_bytes.clone(),
    )
        .into_response()
}

#[cfg(unix)]
fn build_test_python_archive() -> anyhow::Result<Vec<u8>> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tar::{Builder, Header};

    let mut tar_bytes = Vec::new();
    {
        let encoder = GzEncoder::new(&mut tar_bytes, Compression::default());
        let mut builder = Builder::new(encoder);

        let root = test_asset_name().trim_end_matches(".tar.gz");
        let bin_dir = format!("{root}/bin");
        let python_path = format!("{bin_dir}/python3.12");

        let mut root_header = Header::new_gnu();
        root_header.set_entry_type(tar::EntryType::Directory);
        root_header.set_mode(0o755);
        root_header.set_size(0);
        root_header.set_cksum();
        builder.append_data(&mut root_header, root, std::io::empty())?;

        let mut bin_header = Header::new_gnu();
        bin_header.set_entry_type(tar::EntryType::Directory);
        bin_header.set_mode(0o755);
        bin_header.set_size(0);
        bin_header.set_cksum();
        builder.append_data(&mut bin_header, &bin_dir, std::io::empty())?;

        let script = b"#!/bin/sh\necho 'Python 3.12.13'\n";
        let mut python_header = Header::new_gnu();
        python_header.set_entry_type(tar::EntryType::Regular);
        python_header.set_mode(0o755);
        python_header.set_size(script.len() as u64);
        python_header.set_cksum();
        builder.append_data(&mut python_header, &python_path, &script[..])?;

        builder.into_inner()?.finish()?;
    }
    Ok(tar_bytes)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn test_asset_name() -> &'static str {
    "cpython-3.12.13+20260510-aarch64-apple-darwin-install_only.tar.gz"
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn test_asset_name() -> &'static str {
    "cpython-3.12.13+20260510-x86_64-apple-darwin-install_only.tar.gz"
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn test_asset_name() -> &'static str {
    "cpython-3.12.13+20260510-x86_64-unknown-linux-gnu-install_only.tar.gz"
}

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
fn test_asset_name() -> &'static str {
    "cpython-3.12.13+20260510-aarch64-unknown-linux-gnu-install_only.tar.gz"
}

/// GH-5047: a warm restart is a fresh process, so `try_cached` is empty. The
/// durable probe must still recover readiness from a prior managed install on
/// disk — otherwise `is_done` reports "not ready" every launch and the
/// harness-init overlay re-appears.
#[cfg(unix)]
#[tokio::test]
async fn probe_installed_true_from_disk_after_simulated_restart() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_root = tmp.path().join("runtime-python");
    // A prior managed install, extracted under the cache root.
    let bin_dir = cache_root.join("cpython-3.12.13-managed").join("bin");
    std::fs::create_dir_all(&bin_dir).expect("mkdir install");
    let python_path = bin_dir.join("python3.12");
    std::fs::write(&python_path, b"#!/bin/sh\necho 'Python 3.12.13'\n").expect("write python stub");
    std::fs::set_permissions(&python_path, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let mut cfg = RuntimePythonConfig::default();
    cfg.enabled = true;
    cfg.cache_dir = cache_root.display().to_string();
    cfg.prefer_system = false; // force the managed-scan path — no host dependency

    let bootstrap = PythonBootstrap::new(cfg);
    assert!(
        bootstrap.try_cached().is_none(),
        "precondition: process-local cache is empty right after a restart"
    );
    let resolved = bootstrap.probe_installed().await;
    assert!(
        resolved.is_some(),
        "durable probe should recover the managed python install from disk"
    );
    assert_eq!(resolved.unwrap().source, PythonSource::Managed);
}

/// A fresh machine (empty cache, no install) must report "not installed" so a
/// genuine first-run download still runs and the overlay still shows.
#[tokio::test]
async fn probe_installed_none_when_nothing_on_disk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut cfg = RuntimePythonConfig::default();
    cfg.enabled = true;
    cfg.cache_dir = tmp.path().join("runtime-python").display().to_string();
    cfg.prefer_system = false;
    let bootstrap = PythonBootstrap::new(cfg);
    assert!(
        bootstrap.probe_installed().await.is_none(),
        "no on-disk install → provisioning still required"
    );
}

/// A disabled runtime is "nothing to provision", not "installed".
#[tokio::test]
async fn probe_installed_none_when_disabled() {
    let mut cfg = RuntimePythonConfig::default();
    cfg.enabled = false;
    let bootstrap = PythonBootstrap::new(cfg);
    assert!(bootstrap.probe_installed().await.is_none());
}
