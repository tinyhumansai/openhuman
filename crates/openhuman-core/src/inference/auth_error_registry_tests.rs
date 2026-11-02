use super::*;
use std::sync::Mutex;

// The registry is process-global, so these tests would otherwise race:
// one test's `reset_for_tests` can wipe another's entries mid-run. Serialize
// them on a shared lock (poison-tolerant) so each runs against a clean,
// exclusive registry.
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn with_test_lock(future: impl std::future::Future<Output = ()>) {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("create current-thread test runtime")
        .block_on(future);
}

#[test]
fn record_returns_true_only_on_first_episode() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    assert!(record("openrouter", 401), "first record is a new episode");
    assert!(
        !record("openrouter", 401),
        "repeat record of same provider does not re-notify"
    );
    // A different provider is its own episode.
    assert!(record("openai", 403));
    reset_for_tests();
}

#[test]
fn an_unscoped_saas_auth_error_is_not_attributed_to_every_profile() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    let scope = current_scope_in(true, None);
    assert!(matches!(scope, CurrentScope::Unresolved(_)));
    assert!(!record_in_scope("saas-unscoped-provider", 401, scope));
    assert!(
        snapshot().is_empty(),
        "an unresolved SaaS failure must not enter the process-wide registry"
    );

    let workspace = tempfile::tempdir().expect("profile workspace");
    let (errors, has_unscoped) = snapshot_for_scope(
        workspace.path(),
        &workspace.path().join("config.toml"),
        None,
    );
    assert!(errors.is_empty(), "an unscoped SaaS failure has no owner");
    assert!(!has_unscoped, "a SaaS failure is never marked single-user");
    assert!(
        !origins()
            .lock()
            .unwrap()
            .contains_key("saas-unscoped-provider"),
        "the unresolved call must not create a process-wide origin"
    );
    reset_for_tests();
}

#[test]
fn clear_removes_entry_and_rearms_latch() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    assert!(record("openrouter", 401));
    assert!(clear("openrouter"), "entry was present");
    assert!(!clear("openrouter"), "second clear is a no-op");
    // After clear, the next failure is a new episode again.
    assert!(record("openrouter", 401), "latch re-armed after clear");
    reset_for_tests();
}

#[test]
fn snapshot_is_sorted_and_carries_actionable_message() {
    let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_for_tests();
    record("openrouter", 401);
    record("anthropic", 401);
    let snap = snapshot();
    assert_eq!(snap.len(), 2);
    assert_eq!(snap[0].provider, "anthropic");
    assert_eq!(snap[1].provider, "openrouter");
    assert!(snap[1].message.contains("openrouter"));
    assert!(snap[1].message.contains("Connections → API keys → LLM"));
    reset_for_tests();
}

#[test]
fn a_workspace_only_origin_is_not_assigned_to_a_different_config_root() {
    with_test_lock(async {
        reset_for_tests();
        let workspace = tempfile::tempdir().expect("shared workspace");
        let config_root_a = tempfile::tempdir().expect("config root A");
        let config_root_b = tempfile::tempdir().expect("config root B");
        let config_path_b = config_root_b.path().join("config.toml");
        let context = crate::core::runtime::CoreContext::for_test(
            crate::core::runtime::DomainSet::full(),
            Some(workspace.path().to_path_buf()),
        );

        crate::core::runtime::CoreContext::scope(context, async {
            assert!(crate::core::runtime::CoreContext::current_embedder_config().is_none());
            assert!(record("workspace-only-provider", 401));
        })
        .await;

        let (selected_errors, has_unattributed_error) =
            snapshot_for_scope(workspace.path(), &config_path_b, None);
        assert!(
            selected_errors.is_empty(),
            "a workspace-only observation cannot be assigned to config root B"
        );
        assert!(
            has_unattributed_error,
            "the existing process error remains visible as unattributed"
        );

        // Keep both roots live through the assertion; their distinct paths are
        // the ambiguity this regression protects.
        assert_ne!(config_root_a.path(), config_root_b.path());
        reset_for_tests();
    });
}

#[test]
fn workspace_rebind_updates_or_clears_the_config_origin_hint_locally() {
    with_test_lock(async {
        reset_for_tests();
        let workspace = tempfile::tempdir().expect("shared workspace");
        let config_root_a = tempfile::tempdir().expect("config root A");
        let config_root_b = tempfile::tempdir().expect("config root B");
        let config_path_a = config_root_a.path().join("config.toml");
        let config_path_b = config_root_b.path().join("config.toml");
        let mut config_a = crate::config::Config::default();
        config_a.workspace_dir = workspace.path().to_path_buf();
        config_a.config_path = config_path_a.clone();
        let context_a = crate::core::runtime::CoreContext::for_test_with_config(
            crate::core::runtime::DomainSet::full(),
            config_a,
        );
        let derived_a = context_a.for_agent("in-flight-agent-a");

        crate::core::runtime::CoreContext::scope(context_a.clone(), async {
            assert!(record("config-a-provider", 401));
        })
        .await;
        context_a
            .rebind_workspace(workspace.path(), Some(&config_path_b))
            .expect("rebind config origin");
        crate::core::runtime::CoreContext::scope(context_a.clone(), async {
            assert!(record("config-b-provider", 403));
        })
        .await;
        crate::core::runtime::CoreContext::scope(derived_a, async {
            assert!(record("in-flight-config-a-provider", 401));
        })
        .await;

        let (errors_for_a, _) = snapshot_for_scope(workspace.path(), &config_path_a, None);
        let (errors_for_b, _) = snapshot_for_scope(workspace.path(), &config_path_b, None);
        assert_eq!(errors_for_a.len(), 2);
        assert_eq!(errors_for_b.len(), 1);

        context_a
            .rebind_workspace(workspace.path(), None)
            .expect("clear unknown config origin");
        crate::core::runtime::CoreContext::scope(context_a, async {
            assert!(record("unknown-config-provider", 401));
        })
        .await;
        let (errors_for_a, has_unattributed) =
            snapshot_for_scope(workspace.path(), &config_path_a, None);
        assert_eq!(errors_for_a.len(), 2);
        assert!(has_unattributed);
        reset_for_tests();
    });
}

#[test]
fn normal_boot_without_preloaded_config_uses_the_resolved_config_path() {
    const CHILD_MARKER: &str = "OPENHUMAN_AUTH_ORIGIN_NORMAL_BOOT_CHILD";
    if std::env::var_os(CHILD_MARKER).is_none() {
        let home = tempfile::tempdir().expect("isolated test home");
        let openhuman_dir = home.path().join(".openhuman");
        let config_dir = crate::config::pre_login_user_dir(&openhuman_dir);
        std::fs::create_dir_all(&config_dir).expect("create isolated config directory");
        std::fs::write(
            config_dir.join("config.toml"),
            "[storage]\nurl = \"memory\"\n\n[secrets]\nencrypt = false\n",
        )
        .expect("write isolated normal-boot config");

        let output = std::process::Command::new(
            std::env::current_exe().expect("test executable path"),
        )
        .args([
            "--exact",
            "inference::auth_error_registry::tests::normal_boot_without_preloaded_config_uses_the_resolved_config_path",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("OPENHUMAN_APP_ENV", "production")
        .env_remove("VITE_OPENHUMAN_APP_ENV")
        .env_remove("OPENHUMAN_WORKSPACE")
        .env_remove("OPENHUMAN_KEYRING_MASTER_KEY")
        .env_remove("OPENHUMAN_KEYRING_MASTER_KEY_FILE")
        .output()
        .expect("spawn isolated normal-boot test process");
        assert!(
            output.status.success(),
            "isolated normal-boot child failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    with_test_lock(async {
        reset_for_tests();
        assert!(crate::core::runtime::CoreContext::current().is_none());
        let token = crate::core::runtime::TokenSource::Fixed(std::sync::Arc::new(
            "auth-origin-test-token".to_owned(),
        ));
        let (context, _, config) = crate::core::runtime::CoreContext::init_with_boot_mode(
            crate::core::types::HostKind::Library,
            &token,
            crate::core::runtime::DomainSet::none(),
            crate::tools::toolpacks::ToolGroups::none(),
            None,
            crate::core::runtime::CoreBootMode::Normal,
        )
        .await
        .expect("normal boot with config discovery");
        let config = config.expect("normal boot config loaded from disk");
        let config_path_a = config.config_path.clone();
        let config_root_b = tempfile::tempdir().expect("selected config root B");
        let config_path_b = config_root_b.path().join("config.toml");
        assert_eq!(
            config_path_a,
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .expect("isolated HOME")
                .join(".openhuman/users/local/config.toml")
        );

        crate::core::runtime::CoreContext::scope(context, async {
            assert!(crate::core::runtime::CoreContext::current_embedder_config().is_none());
            assert!(record("normal-boot-provider", 401));
        })
        .await;

        let (errors_for_a, unattributed_for_a) =
            snapshot_for_scope(&config.workspace_dir, &config_path_a, None);
        let (errors_for_b, unattributed_for_b) =
            snapshot_for_scope(&config.workspace_dir, &config_path_b, None);
        assert_eq!(errors_for_a.len(), 1);
        assert!(!unattributed_for_a);
        assert!(errors_for_b.is_empty());
        assert!(!unattributed_for_b);
        reset_for_tests();
    });
}

/// One process can serve several workspaces/tenants. The settings RPC must
/// list only the caller's own rejected keys, and one tenant re-keying a
/// provider must not clear another tenant's notice for it.
#[test]
fn auth_errors_stay_in_the_scope_that_recorded_them() {
    with_test_lock(async {
        use crate::core::runtime::{CoreContext, DomainSet};
        reset_for_tests();
        let workspace_a = tempfile::tempdir().expect("workspace A");
        let workspace_b = tempfile::tempdir().expect("workspace B");
        let context_a = CoreContext::for_test(DomainSet::full(), Some(workspace_a.path().into()));
        let context_b = CoreContext::for_test(DomainSet::full(), Some(workspace_b.path().into()));
        let rpc_providers = |context: std::sync::Arc<CoreContext>| {
            CoreContext::scope(context, async {
                let outcome = crate::inference::ops::inference_provider_auth_errors()
                    .await
                    .expect("auth errors rpc");
                outcome.value["errors"]
                    .as_array()
                    .expect("errors array")
                    .iter()
                    .map(|error| {
                        (
                            error["provider"].as_str().unwrap_or_default().to_owned(),
                            error["status"].as_u64().unwrap_or_default(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
        };

        CoreContext::scope(context_a.clone(), async {
            record("tenant-a-only", 401);
            record("shared-provider", 401);
        })
        .await;
        CoreContext::scope(context_b.clone(), async {
            record("shared-provider", 403);
        })
        .await;

        // Each scope sees its own failures, with its own status.
        assert_eq!(
            rpc_providers(context_b.clone()).await,
            [("shared-provider".to_owned(), 403)]
        );
        assert_eq!(
            rpc_providers(context_a.clone()).await,
            [
                ("shared-provider".to_owned(), 401),
                ("tenant-a-only".to_owned(), 401)
            ]
        );

        CoreContext::scope(context_a.clone(), async {
            clear("shared-provider");
        })
        .await;
        assert_eq!(
            rpc_providers(context_b).await,
            [("shared-provider".to_owned(), 403)],
            "tenant A re-keying cleared tenant B's notice"
        );
        assert_eq!(
            rpc_providers(context_a).await,
            [("tenant-a-only".to_owned(), 401)]
        );
        reset_for_tests();
    });
}

/// The doctor's scope read gets the status that scope recorded, not the one a
/// later failure for the same provider in another config root left behind.
#[test]
fn a_scope_read_returns_that_scopes_own_status() {
    with_test_lock(async {
        reset_for_tests();
        let workspace = tempfile::tempdir().expect("shared workspace");
        let config_root_a = tempfile::tempdir().expect("config root A");
        let config_root_b = tempfile::tempdir().expect("config root B");
        let config_path_a = config_root_a.path().join("config.toml");
        let config_path_b = config_root_b.path().join("config.toml");
        let mut config = crate::config::Config::default();
        config.workspace_dir = workspace.path().to_path_buf();
        config.config_path = config_path_a.clone();
        let context = crate::core::runtime::CoreContext::for_test_with_config(
            crate::core::runtime::DomainSet::full(),
            config,
        );
        crate::core::runtime::CoreContext::scope(context.clone(), async {
            record("shared-provider", 401);
        })
        .await;
        context
            .rebind_workspace(workspace.path(), Some(&config_path_b))
            .expect("rebind config origin");
        crate::core::runtime::CoreContext::scope(context, async {
            record("shared-provider", 403);
        })
        .await;

        let status = |config_path: &std::path::Path| {
            let (errors, _) = snapshot_for_scope(workspace.path(), config_path, None);
            errors.iter().map(|error| error.status).collect::<Vec<_>>()
        };
        assert_eq!(status(&config_path_a), [401]);
        assert_eq!(status(&config_path_b), [403]);
        reset_for_tests();
    });
}
