use super::*;
use tinymemory_api::{ConsolidateRequest, ItemKind, Namespace, Reach};
use tinymemory_tools::BackgroundJob;

async fn open_workspace_for_profile(
    workspace: &std::path::Path,
    backend: &std::sync::Arc<dyn crate::storage::StorageBackend>,
    scope: &crate::storage::Scope,
    profile: &str,
) -> crate::storage::state_kv::StateKv {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;

    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.to_path_buf();
    let context = CoreContext::for_test(DomainSet::full(), Some(workspace.to_path_buf()))
        .derive_with(
            ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).profile(profile),
        );
    CoreContext::scope(context, async {
        super::open_workspace_quarantining_on::<Vec<crate::memory::deletion::PendingDeletion>>(
            workspace,
            backend,
            scope,
            "pending_deletions",
        )
        .unwrap()
    })
    .await
}

#[tokio::test]
async fn host_memory_state_adapters_use_the_state_kv_port() {
    let tmp = tempfile::tempdir().unwrap();
    let config = crate::memory::test_fixtures::config_in(&tmp);
    let workspace = &config.workspace_dir;

    crate::memory::deletion::enqueue(
        workspace,
        crate::memory::deletion::PendingDeletion::Thread {
            thread_id: "thread-1".into(),
        },
    );
    crate::memory::sources::state::update(workspace, "src-1", |_| {});
    crate::memory::layout_migration::state::save(workspace, &Default::default()).unwrap();
    crate::memory::lifecycle::jobs::enqueue(
        &config,
        &Namespace::ROOT,
        vec![BackgroundJob::BuildBeliefs {
            request: ConsolidateRequest::new(Reach::exact(Namespace::agent("a")))
                .kinds([ItemKind::Conversation]),
        }],
    )
    .await;

    for name in [
        "pending_deletions",
        "sources_state",
        "layout_migration",
        "jobs",
    ] {
        let stored = if name == "pending_deletions" {
            load_workspace::<serde_json::Value>(workspace, name)
        } else {
            load::<serde_json::Value>(workspace, name)
        };
        assert!(
            stored.unwrap().is_some(),
            "memory state value {name} is stored through the KV port"
        );
    }
}

#[test]
fn workspace_scopes_keep_retry_queues_stable_and_isolated_on_shared_storage() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let backend = crate::storage::test_support::memory_backend();
    let first_scope = super::workspace_scope(first.path()).unwrap();
    let second_scope = super::workspace_scope(second.path()).unwrap();
    let first_storage = backend.for_scope(&first_scope).unwrap();
    let second_storage = backend.for_scope(&second_scope).unwrap();
    let first_state = crate::storage::state_kv::StateKv::over(&first_storage, "memory", "state");

    first_state
        .put("pending_deletions", &vec!["thread-a".to_string()])
        .unwrap();

    let first_reopened = crate::storage::state_kv::StateKv::over(&first_storage, "memory", "state");
    let second_state = crate::storage::state_kv::StateKv::over(&second_storage, "memory", "state");
    assert_eq!(
        first_reopened
            .get::<Vec<String>>("pending_deletions")
            .unwrap(),
        Some(vec!["thread-a".to_string()])
    );
    assert!(
        second_state
            .get::<Vec<String>>("pending_deletions")
            .unwrap()
            .is_none(),
        "a different workspace cannot read the queue"
    );
}

#[test]
fn workspace_scope_is_stable_before_creation_and_across_profile_layouts() {
    use crate::config::schema::profile_layout::{LayoutVersion, ProfileLayout};

    let tmp = tempfile::tempdir().unwrap();
    let absent = tmp.path().join("created-on-first-use");
    let before = super::workspace_scope(&absent).unwrap();
    std::fs::create_dir_all(&absent).unwrap();
    assert_eq!(super::workspace_scope(&absent).unwrap(), before);

    let profile_dir = tmp.path().join("users").join("alice");
    std::fs::create_dir_all(&profile_dir).unwrap();
    let v1 = ProfileLayout::with_version(profile_dir.clone(), LayoutVersion::V1);
    let v1_scope = super::workspace_scope(&v1.workspace_dir).unwrap();
    std::fs::create_dir_all(&v1.workspace_dir).unwrap();
    LayoutVersion::mark_v2(&profile_dir).unwrap();
    let v2 = ProfileLayout::with_version(profile_dir, LayoutVersion::V2);
    assert_eq!(super::workspace_scope(&v2.workspace_dir).unwrap(), v1_scope);
}

#[tokio::test]
async fn first_workspace_open_transfers_only_its_old_profile_queue_and_rechecks_after_marker() {
    use crate::config::schema::profile_layout::{LayoutVersion, ProfileLayout};
    use crate::memory::deletion::PendingDeletion;

    let tmp = tempfile::tempdir().unwrap();
    let alice_dir = tmp.path().join("users").join("alice");
    let bob_dir = tmp.path().join("users").join("bob");
    std::fs::create_dir_all(&alice_dir).unwrap();
    std::fs::create_dir_all(&bob_dir).unwrap();
    LayoutVersion::mark_v2(&alice_dir).unwrap();
    LayoutVersion::mark_v2(&bob_dir).unwrap();
    let alice_workspace = ProfileLayout::with_version(alice_dir, LayoutVersion::V2).workspace_dir;
    let bob_workspace = ProfileLayout::with_version(bob_dir, LayoutVersion::V2).workspace_dir;

    let backend_fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend = std::sync::Arc::clone(&backend_fixture.backend);
    let alice_old = crate::storage::state_kv::StateKv::on_backend(
        &backend,
        &crate::storage::scope_for_profile("alice"),
        "memory",
        "state",
    )
    .unwrap();
    let bob_old = crate::storage::state_kv::StateKv::on_backend(
        &backend,
        &crate::storage::scope_for_profile("bob"),
        "memory",
        "state",
    )
    .unwrap();
    let local_old = crate::storage::state_kv::StateKv::on_backend(
        &backend,
        &crate::storage::Scope::local(),
        "memory",
        "state",
    )
    .unwrap();
    let alice_item = PendingDeletion::Thread {
        thread_id: "alice-owed".into(),
    };
    let local_item = PendingDeletion::Thread {
        thread_id: "local-owed".into(),
    };
    let bob_item = PendingDeletion::Thread {
        thread_id: "bob-owed".into(),
    };
    alice_old
        .put("pending_deletions", &vec![alice_item.clone()])
        .unwrap();
    local_old
        .put("pending_deletions", &vec![local_item.clone()])
        .unwrap();
    bob_old
        .put("pending_deletions", &vec![bob_item.clone()])
        .unwrap();

    let alice_scope = super::workspace_scope(&alice_workspace).unwrap();
    let alice_target =
        open_workspace_for_profile(&alice_workspace, &backend, &alice_scope, "alice").await;
    assert_eq!(
        alice_target
            .get::<Vec<PendingDeletion>>("pending_deletions")
            .unwrap()
            .unwrap()
            .into_iter()
            .collect::<std::collections::HashSet<_>>(),
        [alice_item.clone(), local_item.clone()]
            .into_iter()
            .collect()
    );
    assert!(
        local_old
            .get::<Vec<PendingDeletion>>("pending_deletions")
            .unwrap()
            .unwrap_or_default()
            .is_empty(),
        "the previous local-scope queue moves to the workspace scope"
    );
    assert!(
        bob_old
            .get::<Vec<PendingDeletion>>("pending_deletions")
            .unwrap()
            .is_some(),
        "opening Alice's workspace does not inspect or clear Bob's tenant scope"
    );

    alice_target.delete("pending_deletions").unwrap();
    open_workspace_for_profile(&alice_workspace, &backend, &alice_scope, "alice").await;
    assert!(
        alice_target
            .get::<Vec<PendingDeletion>>("pending_deletions")
            .unwrap()
            .is_none(),
        "a completed transfer does not resurrect a queue drained after restart"
    );

    let late_item = PendingDeletion::Thread {
        thread_id: "alice-late-legacy-write".into(),
    };
    alice_old
        .put("pending_deletions", &vec![late_item.clone()])
        .unwrap();
    open_workspace_for_profile(&alice_workspace, &backend, &alice_scope, "alice").await;
    assert_eq!(
        alice_target
            .get::<Vec<PendingDeletion>>("pending_deletions")
            .unwrap(),
        Some(vec![late_item]),
        "a per-source marker does not hide a later old-scope write"
    );

    let bob_scope = super::workspace_scope(&bob_workspace).unwrap();
    let bob_target = open_workspace_for_profile(&bob_workspace, &backend, &bob_scope, "bob").await;
    assert_eq!(
        bob_target
            .get::<Vec<PendingDeletion>>("pending_deletions")
            .unwrap(),
        Some(vec![bob_item])
    );
}

#[tokio::test]
async fn a_matching_active_agent_scope_is_a_legacy_source() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;

    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("custom-workspace");
    let mut config = crate::config::Config::default();
    config.workspace_dir = workspace.clone();
    let context = CoreContext::for_test(DomainSet::full(), Some(workspace.clone())).derive_with(
        ContextOverlay::new(config, DomainSet::full(), ToolGroups::none()).session_agent("agent-a"),
    );
    let backend_fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend = std::sync::Arc::clone(&backend_fixture.backend);
    let old_scope = crate::storage::scope_for_agent("agent-a");
    let old =
        crate::storage::state_kv::StateKv::on_backend(&backend, &old_scope, "memory", "state")
            .unwrap();
    let item = crate::memory::deletion::PendingDeletion::Thread {
        thread_id: "agent-owed".into(),
    };
    old.put("pending_deletions", &vec![item.clone()]).unwrap();
    let target_scope = super::workspace_scope(&workspace).unwrap();
    let target = super::open_workspace_quarantining_on::<
        Vec<crate::memory::deletion::PendingDeletion>,
    >(&workspace, &backend, &target_scope, "pending_deletions")
    .unwrap();

    let migrated = CoreContext::scope(context.clone(), async {
        let includes_own_scope = super::legacy_queue_scopes(&workspace, true).contains(&old_scope);
        includes_own_scope
            && super::open_workspace_quarantining_on::<
                Vec<crate::memory::deletion::PendingDeletion>,
            >(&workspace, &backend, &target_scope, "pending_deletions")
            .is_ok()
    })
    .await;
    assert!(migrated);
    assert_eq!(
        target
            .get::<Vec<crate::memory::deletion::PendingDeletion>>("pending_deletions")
            .unwrap(),
        Some(vec![item])
    );

    let other_workspace = tmp.path().join("other-workspace");
    let other_scope = super::workspace_scope(&other_workspace).unwrap();
    let other_target =
        super::open_workspace_quarantining_on::<Vec<crate::memory::deletion::PendingDeletion>>(
            &other_workspace,
            &backend,
            &other_scope,
            "pending_deletions",
        )
        .unwrap();
    CoreContext::scope(context, async {
        super::migrate_existing_queue_scopes(
            &other_workspace,
            &backend,
            &other_scope,
            &other_target,
            false,
        )
        .unwrap();
    })
    .await;
    assert!(
        other_target
            .get::<Vec<crate::memory::deletion::PendingDeletion>>("pending_deletions")
            .unwrap()
            .is_none(),
        "an active agent scope is not copied into a different workspace"
    );
}

#[test]
fn layout_import_places_legacy_retry_queue_in_its_workspace_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let queue_file = super::legacy_path(tmp.path(), "pending_deletions");
    std::fs::create_dir_all(queue_file.parent().unwrap()).unwrap();
    std::fs::write(
        &queue_file,
        serde_json::to_vec(&vec![crate::memory::deletion::PendingDeletion::Thread {
            thread_id: "thread-legacy".into(),
        }])
        .unwrap(),
    )
    .unwrap();

    let backend_fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend = std::sync::Arc::clone(&backend_fixture.backend);
    let local = backend.for_scope(&crate::storage::Scope::local()).unwrap();
    let local_queue = crate::storage::state_kv::StateKv::over(&local, "memory", "state");
    local_queue
        .put(
            "pending_deletions",
            &vec![crate::memory::deletion::PendingDeletion::Thread {
                thread_id: "thread-previously-imported".into(),
            }],
        )
        .unwrap();
    super::import_all(&backend, tmp.path()).unwrap();

    let workspace_scope = super::workspace_scope(tmp.path()).unwrap();
    let workspace = backend.for_scope(&workspace_scope).unwrap();
    let queue = crate::storage::state_kv::StateKv::over(&workspace, "memory", "state");
    assert_eq!(
        queue
            .get::<Vec<crate::memory::deletion::PendingDeletion>>("pending_deletions")
            .unwrap(),
        Some(vec![
            crate::memory::deletion::PendingDeletion::Thread {
                thread_id: "thread-legacy".into(),
            },
            crate::memory::deletion::PendingDeletion::Thread {
                thread_id: "thread-previously-imported".into(),
            },
        ])
    );

    assert!(
        local_queue
            .get::<Vec<crate::memory::deletion::PendingDeletion>>("pending_deletions")
            .unwrap()
            .unwrap_or_default()
            .is_empty(),
        "the migration must not strand a duplicate queue in local scope"
    );
}

#[test]
fn state_values_do_not_leak_between_workspaces() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let first_config = crate::memory::test_fixtures::config_in(&first);
    let second_config = crate::memory::test_fixtures::config_in(&second);

    save(&first_config.workspace_dir, "isolation", &"first workspace").unwrap();

    assert_eq!(
        load::<String>(&first_config.workspace_dir, "isolation").unwrap(),
        Some("first workspace".to_string())
    );
    assert!(
        load::<String>(&second_config.workspace_dir, "isolation")
            .unwrap()
            .is_none(),
        "a second workspace must not read the first workspace's state"
    );
}
