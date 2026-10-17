//! The host and the real compiled `tinymemes` module over the bus.
//!
//! Runs where a module library exists (`node scripts/modules/build.mjs
//! tinymemes` with `OPENHUMAN_MODULE_SOURCE_DIR`, or the released modules the CI
//! installer stages under `OPENHUMAN_MODULE_PATH`); skipped, with a message,
//! otherwise. The library is loaded through a path override, the same way a
//! developer points a host at a local build.

use super::*;
use service::MODULE_ID;
use tinymemes_bus::IndexKind;

fn config_in(dir: &std::path::Path, artifact: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.config_path = dir.join("config.toml");
    config.workspace_dir = dir.join("workspace");
    config.secrets.encrypt = false;
    config.components.memes = true;
    config.modules.allow_download = false;
    config
        .modules
        .overrides
        .push(crate::config::schema::ModuleOverride::with_path(
            MODULE_ID,
            artifact.to_str().unwrap(),
        ));
    config
}

fn artifact() -> Option<std::path::PathBuf> {
    let artifact = crate::modules::local_artifact(MODULE_ID);
    if artifact.is_none() {
        eprintln!(
            "SKIPPED (not run, not asserted): no tinymemes library. Run \
             `node scripts/modules/build.mjs tinymemes` and set OPENHUMAN_MODULE_SOURCE_DIR, \
             or point OPENHUMAN_MODULE_PATH at the released modules"
        );
    }
    artifact
}

fn request(reply: &str) -> RemixRequest {
    serde_json::from_value(serde_json::json!({
        "conversation": [{ "role": "user", "text": "ok the report is done", "id": "u-1" }],
        "reply": reply,
        "region": "global",
        "timeout_ms": 20_000,
        "reply_id": "run-1:reply",
    }))
    .unwrap()
}

fn write_legacy(config: &Config, kind_file: &str, content: &str) {
    let dir = config.workspace_dir.join("tinymemes");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(kind_file), content).unwrap();
}

/// With no Jev, no inference provider and no search on this host, the module
/// still answers: it asks, is told it is unsupported, and fails open to the
/// original reply. Its document reads reach the host's callbacks with its
/// correlation id, which is visible as the legacy file being adopted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_real_module_fails_open_and_reads_its_documents_through_the_host() {
    let Some(artifact) = artifact() else { return };
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path(), &artifact);
    write_legacy(&config, "remixed.json", "[\"old-1\"]");

    let response = service::remix(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        request("That's done."),
    )
    .await
    .expect("the module answered");
    assert!(!response.remixed, "{response:?}");
    assert_eq!(response.reply, "That's done.");
    // The module reported the request finished, so nothing is kept for it.
    assert_eq!(callbacks::live_for(&config.workspace_dir), 0);

    let legacy = config.workspace_dir.join("tinymemes/remixed.json");
    assert!(
        !legacy.exists(),
        "the module's LoadIndex adopted the old file"
    );
    assert_eq!(
        docs::load(&config, IndexKind::Remixed).unwrap().as_deref(),
        Some("[\"old-1\"]")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disabled_request_passes_through_without_a_callback() {
    let Some(artifact) = artifact() else { return };
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(dir.path(), &artifact);
    write_legacy(&config, "remixed.json", "[\"untouched\"]");
    let mut disabled = request("hello");
    disabled.enabled = false;
    let response = service::remix(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        disabled,
    )
    .await
    .unwrap();
    assert_eq!(response.reply, "hello");
    assert_eq!(response.skipped, Some(SkipReason::Disabled));
    assert_eq!(
        callbacks::live_for(&config.workspace_dir),
        0,
        "a disabled request keeps no registration"
    );
    assert!(
        config.workspace_dir.join("tinymemes/remixed.json").exists(),
        "a disabled request reads nothing"
    );
}

/// Two workspaces remix at once. Each callback is answered for the workspace
/// of its own correlation id, and neither waits behind a process-wide gate.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_remixes_for_two_workspaces_stay_in_their_own_documents() {
    let Some(artifact) = artifact() else { return };
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (
        config_in(dir_a.path(), &artifact),
        config_in(dir_b.path(), &artifact),
    );
    write_legacy(&a, "remixed.json", "[\"from-a\"]");
    write_legacy(&b, "remixed.json", "[\"from-b\"]");

    let host = crate::core::runtime::DispatchCtx::host_default();
    let (ra, rb) = tokio::join!(
        service::remix(&host, &a, request("reply a")),
        service::remix(&host, &b, request("reply b"))
    );
    assert_eq!(ra.unwrap().reply, "reply a");
    assert_eq!(rb.unwrap().reply, "reply b");

    assert_eq!(
        docs::load(&a, IndexKind::Remixed).unwrap().as_deref(),
        Some("[\"from-a\"]")
    );
    assert_eq!(
        docs::load(&b, IndexKind::Remixed).unwrap().as_deref(),
        Some("[\"from-b\"]")
    );
}
