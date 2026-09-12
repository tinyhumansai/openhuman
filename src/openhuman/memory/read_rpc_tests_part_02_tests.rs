use super::*;

#[tokio::test]
#[ignore = "needs a built tinymemory module (OPENHUMAN_MODULE_PATH) and its own process: \
chunk detail is read through the bound driver, not the in-process engine"]
async fn read_chunk_row_returns_none_for_missing_chunk() {
    let (_tmp, _cfg) = test_config();
    assert!(read_chunk_row("missing-chunk").await.unwrap().is_none());
}

#[test]
fn display_name_unslugs_email_thread_with_user_hint() {
    let name = display_name_for_source(
        "gmail:alice@example.com|bob@example.com",
        Some("alice@example.com"),
    );
    assert_eq!(name, "bob@example.com");
}

#[test]
fn display_name_falls_back_to_arrow_when_user_unknown() {
    let name = display_name_for_source("gmail:alice@example.com|bob@example.com", None);
    assert!(name.contains("alice@example.com"));
    assert!(name.contains("bob@example.com"));
    assert!(name.contains("↔"));
}

#[test]
fn display_name_strips_platform_prefix() {
    assert_eq!(
        display_name_for_source("slack:#engineering", None),
        "#engineering"
    );
}

#[test]
fn display_name_handles_multiple_participants_and_trimmed_hint() {
    let name = display_name_for_source(
        "gmail:Alice@Example.com|bob@example.com|carol@example.com",
        Some(" alice@example.com "),
    );
    assert_eq!(name, "bob@example.com, carol@example.com");
}

#[test]
fn display_name_handles_no_prefix() {
    assert_eq!(display_name_for_source("loose-id", None), "loose-id");
}

#[test]
fn sanitize_basename_replaces_windows_illegal_characters() {
    assert_eq!(
        sanitize_basename(r#"chat:slack/#eng\name*?"<>|"#),
        "chat-slack-#eng-name------"
    );
    assert_eq!(sanitize_basename("safe-name.md"), "safe-name.md");
}

#[test]
fn parse_source_kind_str_accepts_known_values_only() {
    assert_eq!(parse_source_kind_str("chat"), Some(SourceKind::Chat));
    assert_eq!(parse_source_kind_str("email"), Some(SourceKind::Email));
    assert_eq!(
        parse_source_kind_str("document"),
        Some(SourceKind::Document)
    );
    assert_eq!(parse_source_kind_str("unknown"), None);
}

#[tokio::test]
async fn obsidian_status_registered_when_override_config_lists_content_root() {
    let (_tmp, cfg) = test_config();
    let content_root = cfg.memory_tree_content_root();
    // A separate dir standing in for a non-standard Obsidian config
    // location, with an obsidian.json that registers the content root.
    let cfg_dir = TempDir::new().unwrap();
    let body = format!(
        "{{ \"vaults\": {{ \"id0\": {{ \"path\": {}, \"open\": true }} }} }}",
        serde_json::to_string(&content_root.to_string_lossy().to_string()).unwrap()
    );
    std::fs::write(cfg_dir.path().join("obsidian.json"), body).unwrap();

    let outcome =
        obsidian_vault_status_rpc(&cfg, Some(cfg_dir.path().to_string_lossy().to_string()))
            .await
            .unwrap();

    assert!(outcome.value.registered);
    assert!(outcome.value.config_found);
    assert_eq!(
        outcome.value.content_root_abs,
        content_root.to_string_lossy().to_string()
    );
    // The log reports the booleans but redacts the absolute path (it
    // embeds the user's home / username).
    assert!(
        outcome.logs[0].contains("registered=true"),
        "log: {}",
        outcome.logs[0]
    );
    assert!(
        !outcome.logs[0].contains(content_root.to_str().unwrap()),
        "log leaked content root: {}",
        outcome.logs[0]
    );
}

#[tokio::test]
async fn obsidian_status_not_registered_for_empty_override_dir() {
    let (_tmp, cfg) = test_config();
    // Empty override dir → no obsidian.json there → content root is not a
    // registered vault. (A temp content root can't be under any real host
    // vault either, so this stays false regardless of the dev machine.)
    let cfg_dir = TempDir::new().unwrap();
    let outcome =
        obsidian_vault_status_rpc(&cfg, Some(cfg_dir.path().to_string_lossy().to_string()))
            .await
            .unwrap();
    assert!(!outcome.value.registered);
}

#[tokio::test]
async fn obsidian_status_blank_override_is_treated_as_none() {
    // A whitespace-only override must be normalized to None rather than
    // resolving to "." and probing a stray local ./obsidian.json. The temp
    // content root isn't under any real host vault, so this stays false.
    let (_tmp, cfg) = test_config();
    let outcome = obsidian_vault_status_rpc(&cfg, Some("   ".to_string()))
        .await
        .unwrap();
    assert!(!outcome.value.registered);
}

/// #4278: both vault RPCs stamp the core host's OS so a frontend attached
/// from a different OS can tell `content_root_abs` is a foreign-host path and
/// must not open/reveal it locally.
#[tokio::test]
async fn vault_rpcs_report_core_host_os() {
    let (_tmp, cfg) = test_config();
    // `vault_health_check_rpc` folds in `pipeline_status_rpc`, which reads
    // through the bound driver. Bind an empty one explicitly: resolving the
    // real driver means loading the compiled module, which a test process
    // can block on rather than fail.
    crate::openhuman::memory::binding::install_diagnostics_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        Default::default(),
        Default::default(),
    );

    let status = obsidian_vault_status_rpc(&cfg, None).await.unwrap();
    assert_eq!(status.value.host_os, std::env::consts::OS);

    let health = vault_health_check_rpc(&cfg, None).await.unwrap();
    assert_eq!(health.value.host_os, std::env::consts::OS);
    assert!(
        !health.value.host_os.is_empty(),
        "host_os must be populated"
    );
}
