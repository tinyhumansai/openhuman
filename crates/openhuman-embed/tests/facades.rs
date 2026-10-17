//! Behaviour of the curated facades a host reaches without a runtime:
//! `artifacts`, `chat_surface`, `identity`, `config` helpers, `modules`, and
//! the compile-status constants. No runtime is built, so these share a process.

use std::time::Duration;

use openhuman_embed::artifacts::{
    create_artifact, finalize_artifact, resolve_ready_file, ArtifactKind, ArtifactStatus, FileRoots,
};
use openhuman_embed::chat_surface::{
    publish_web_channel_event, subscribe_web_channel_events, WebChannelEvent,
};
use openhuman_embed::identity::{peek_credential_user_identity, UserIdentity};

#[tokio::test]
async fn an_artifact_resolves_only_once_ready_and_only_under_its_roots() {
    let workspace = tempfile::tempdir().expect("workspace");
    let files = tempfile::tempdir().expect("files");
    let roots = FileRoots::new(files.path());

    let (meta, path) = create_artifact(
        workspace.path(),
        roots.clone(),
        ArtifactKind::Document,
        "Facade note",
        "md",
        None,
    )
    .await
    .expect("artifact reserved");
    assert!(path.starts_with(files.path()), "{}", path.display());

    let pending = resolve_ready_file(workspace.path(), &roots, &meta.id)
        .await
        .expect_err("a pending artifact does not resolve");
    assert!(pending.contains("not ready"), "{pending}");

    tokio::fs::write(&path, b"hello").await.expect("write file");
    let ready = finalize_artifact(workspace.path(), &meta.id, 5, None)
        .await
        .expect("finalized");
    assert_eq!(ready.status, ArtifactStatus::Ready);
    let resolved = resolve_ready_file(workspace.path(), &roots, &meta.id)
        .await
        .expect("ready artifact resolves");
    assert_eq!(resolved, path);

    // A host that trusts a different folder must not resolve the same record.
    let elsewhere = tempfile::tempdir().expect("elsewhere");
    assert!(
        resolve_ready_file(
            workspace.path(),
            &FileRoots::new(elsewhere.path()),
            &meta.id
        )
        .await
        .is_err(),
        "a file outside the trusted roots is refused"
    );

    tokio::fs::remove_file(&path).await.expect("remove file");
    let missing = resolve_ready_file(workspace.path(), &roots, &meta.id)
        .await
        .expect_err("a deleted file is reported");
    assert!(missing.contains("file missing"), "{missing}");

    assert!(
        resolve_ready_file(workspace.path(), &roots, "no-such-artifact")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn web_channel_events_reach_a_subscriber_with_a_timestamp() {
    let mut events = subscribe_web_channel_events();
    publish_web_channel_event(WebChannelEvent {
        event: "facade_probe".into(),
        thread_id: "thread-facade".into(),
        ..WebChannelEvent::default()
    });
    let received = loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .expect("an event arrives")
            .expect("bus open");
        if event.event == "facade_probe" {
            break event;
        }
    };
    assert_eq!(received.thread_id, "thread-facade");
    assert!(received.ts.is_some(), "the publisher stamps ts");
}

#[test]
fn identity_is_empty_when_nobody_is_signed_in() {
    assert!(peek_credential_user_identity().is_none());
    assert!(UserIdentity::default().is_empty());
    assert!(!UserIdentity {
        id: Some("u1".into()),
        ..UserIdentity::default()
    }
    .is_empty());
}

#[test]
fn the_active_user_marker_reads_back_from_a_root() {
    use openhuman_embed::config::read_active_user_id;

    let root = tempfile::tempdir().expect("root");
    assert_eq!(read_active_user_id(root.path()), None, "no marker yet");

    std::fs::write(
        root.path().join("active_user.toml"),
        "user_id = \"u-facade\"\n",
    )
    .expect("write marker");
    assert_eq!(
        read_active_user_id(root.path()).as_deref(),
        Some("u-facade")
    );
}

#[test]
fn compile_status_constants_are_the_cores_and_usable_in_const_context() {
    // Hosts assert these at compile time (`const _: () = assert!(..)`), so they
    // must stay `const bool`s. Embed's own cargo features do not mirror the
    // core's (the default set comes from `openhuman-core/default`), so the
    // value is compared with the core's, not with a `cfg!`.
    const HTTP: bool = openhuman_embed::HTTP_SERVER_COMPILED_IN;
    const VOICE: bool = openhuman_embed::VOICE_COMPILED_IN;
    assert_eq!(
        HTTP,
        openhuman_core::core::http_server_status::HTTP_SERVER_COMPILED_IN
    );
    assert_eq!(VOICE, openhuman_core::voice::VOICE_COMPILED_IN);
}

#[test]
fn schema_lookup_finds_a_builtin_method_and_rejects_an_unknown_one() {
    let schema = openhuman_embed::schema_for_rpc_method("openhuman.config_get_runtime_flags")
        .expect("built-in controller is registered");
    assert_eq!(schema.namespace, "config");
    assert_eq!(schema.function, "get_runtime_flags");
    assert!(openhuman_embed::schema_for_rpc_method("openhuman.no_such_method").is_none());
}

#[cfg(feature = "modules")]
#[test]
fn the_bundled_releases_dir_is_set_once() {
    use openhuman_embed::modules::{browser, set_bundled_releases_dir};

    assert!(!browser::MODULE_ID.is_empty());
    let first = std::path::PathBuf::from("/nonexistent/embed-facade-first");
    let second = std::path::PathBuf::from("/nonexistent/embed-facade-second");
    assert!(set_bundled_releases_dir(first).is_ok());
    assert_eq!(set_bundled_releases_dir(second.clone()), Err(second));
}
