use super::*;
use crate::core::runtime::DispatchCtx;
use crate::profiles::doctor::DoctorProfileState;
use crate::storage::test_support::memory_backend;
#[cfg(feature = "storage-sqlite")]
use crate::storage::test_support::sqlite_backend;
#[cfg(feature = "storage-sqlite")]
use crate::storage::{CollectionSpec, Scope};
use std::collections::BTreeMap;

fn snapshot_tree(root: &std::path::Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn visit(
        root: &std::path::Path,
        current: &std::path::Path,
        snapshot: &mut BTreeMap<std::path::PathBuf, Vec<u8>>,
    ) {
        let Ok(entries) = std::fs::read_dir(current) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                snapshot.insert(relative.join("."), Vec::new());
                visit(root, &path, snapshot);
            } else if let Ok(contents) = std::fs::read(&path) {
                snapshot.insert(relative, contents);
            }
        }
    }

    let mut snapshot = BTreeMap::new();
    visit(root, root, &mut snapshot);
    snapshot
}

#[tokio::test]
#[cfg(feature = "storage-sqlite")]
async fn inspecting_an_unprovisioned_profile_does_not_provision_or_claim_it() {
    let tmp = tempfile::tempdir().unwrap();
    let fixture = sqlite_backend().unwrap();
    let backend = Arc::clone(&fixture.backend);
    let saas = SaasConfig::new(tmp.path());
    let host = ProfileHost::with_backend(
        saas,
        CoreContext::for_test(DomainSet::full(), None),
        Some(Arc::clone(&backend)),
    )
    .unwrap();
    let cluster = backend
        .for_scope(&Scope::new(crate::storage::lease::CLUSTER_SCOPE).unwrap())
        .unwrap();
    let registry_spec = CollectionSpec::new(crate::profiles::registry::REGISTRY_COLLECTION);
    let lease_spec = CollectionSpec::new(crate::storage::lease::LEASE_COLLECTION);
    let docs = cluster.documents();
    let id = profile("alice");
    let layout = host.layout_of(&id);
    let default_context = CoreContext::default_context();

    // The host declares its registry during construction. Inspection must
    // still leave the profile absent and must not claim its lease.
    assert!(docs.validate_collection_spec(&registry_spec).await.is_ok());
    assert!(docs.validate_collection_spec(&lease_spec).await.is_err());
    assert_eq!(
        host.inspect_for_doctor(&id).await,
        DoctorProfileState::Unprovisioned
    );
    let target = host.target_for_doctor(&id).unwrap();
    assert_eq!(target.profile_id, id);
    assert_eq!(target.context.profile(), Some(id.as_str()));
    assert_eq!(target.config.config_path, layout.config_path);
    assert_eq!(host.open_count(), 0, "inspection must not open or lease it");
    assert!(
        !layout.dir.exists(),
        "inspection must not create profile dirs"
    );
    let default_context_after = CoreContext::default_context();
    match (default_context.as_ref(), default_context_after.as_ref()) {
        (None, None) => {}
        (Some(before), Some(after)) => assert!(Arc::ptr_eq(before, after)),
        _ => panic!("doctor inspection must not publish a process default context"),
    }
    let lease_key = tinystoragedrivers::Key::new(format!(
        "{}:{}",
        crate::storage::lease::LEASE_OWNER,
        id.as_str()
    ))
    .unwrap();
    assert!(
        docs.get(crate::storage::lease::LEASE_COLLECTION, lease_key.as_str())
            .await
            .unwrap()
            .is_none(),
        "inspection must not claim a lease"
    );

    assert!(
        docs.validate_collection_spec(&registry_spec).await.is_ok(),
        "inspection leaves the host's registry schema available"
    );
    assert!(
        docs.validate_collection_spec(&lease_spec).await.is_err(),
        "inspection must not create or migrate the lease schema"
    );
}

#[tokio::test]
async fn selected_doctor_context_keeps_config_storage_credentials_and_provider_binding_per_profile()
{
    let tmp = tempfile::tempdir().unwrap();
    let mut saas = SaasConfig::new(tmp.path());
    saas.max_tool_iterations = Some(7);
    saas.cron.min_interval_secs = 900;
    let host = ProfileHost::new(saas, CoreContext::for_test(DomainSet::full(), None));
    let a = ProfileId::for_user(
        &format!("doctor-a-{}", uuid::Uuid::new_v4().simple()),
        crate::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    let b = ProfileId::for_user(
        &format!("doctor-b-{}", uuid::Uuid::new_v4().simple()),
        crate::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    host.provision(&a).await.unwrap();
    host.provision(&b).await.unwrap();
    let target_a = host.target_for_doctor(&a).unwrap();
    let target_b = host.target_for_doctor(&b).unwrap();

    let profile_store = target_b
        .config
        .config_path
        .parent()
        .unwrap()
        .join("auth-profiles.json");
    std::fs::write(
        &profile_store,
        r#"{"schema_version":1,"active_profiles":{"api-key":"api-key:default"},"profiles":{"api-key:default":{"provider":"api-key","profile_name":"default","kind":"token","token":"doctor-b-only-test-key","metadata":{"kind":"tinyhumans-api-key"}}}}"#,
    )
    .expect("seed profile B's temporary read-only API-key fixture");

    let before_reads = snapshot_tree(tmp.path());
    let a_observed = CoreContext::scope(Arc::clone(&target_a.context), async {
        let selected_context = CoreContext::current().expect("selected profile context");
        let config = selected_context
            .embedder_config()
            .expect("selected profile config");
        (
            config.config_path.clone(),
            crate::storage::current_scope().unwrap(),
            crate::credentials::api_key::get_api_key_read_only(config)
                .await
                .unwrap(),
            crate::credentials::session_state::load_app_session_profile_read_only(config)
                .await
                .unwrap(),
            crate::profiles::provider_settings::ProfileProviderSettings::in_ctx(
                &DispatchCtx::ambient(),
            )
            .is_some(),
        )
    })
    .await;
    let b_observed = CoreContext::scope(Arc::clone(&target_b.context), async {
        let selected_context = CoreContext::current().expect("selected profile context");
        let config = selected_context
            .embedder_config()
            .expect("selected profile config");
        (
            config.config_path.clone(),
            crate::storage::current_scope().unwrap(),
            crate::credentials::api_key::get_api_key_read_only(config)
                .await
                .unwrap(),
            crate::credentials::session_state::load_app_session_profile_read_only(config)
                .await
                .unwrap(),
            crate::profiles::provider_settings::ProfileProviderSettings::in_ctx(
                &DispatchCtx::ambient(),
            )
            .is_some(),
        )
    })
    .await;

    assert_eq!(a_observed.0, target_a.config.config_path);
    assert_eq!(b_observed.0, target_b.config.config_path);
    assert_ne!(a_observed.0, b_observed.0);
    assert_eq!(a_observed.1, crate::storage::scope_for_profile(a.as_str()));
    assert_eq!(b_observed.1, crate::storage::scope_for_profile(b.as_str()));
    assert!(a_observed.2.is_none(), "A must not observe B's API key");
    assert!(a_observed.3.is_none(), "A must not observe B's session");
    assert_eq!(b_observed.2.as_deref(), Some("doctor-b-only-test-key"));
    assert!(b_observed.3.is_none(), "B has no app-session profile");
    assert!(
        a_observed.4 && b_observed.4,
        "each forced config resolves provider settings"
    );
    assert_eq!(target_a.config.agent.max_tool_iterations_override, Some(7));
    assert_eq!(target_b.config.cron.min_interval_secs, 900);
    let storage = memory_backend();
    assert!(storage.for_scope(&a_observed.1).is_ok());
    assert!(storage.for_scope(&b_observed.1).is_ok());
    assert_eq!(
        host.open_count(),
        0,
        "doctor inspection does not open profiles"
    );
    assert_eq!(before_reads, snapshot_tree(tmp.path()));
}

#[tokio::test]
async fn doctor_inspection_reports_an_existing_lease_without_taking_it() {
    let tmp = tempfile::tempdir().unwrap();
    let operator = CoreContext::for_test(DomainSet::full(), None);
    let make_host = |node: &str| {
        let mut saas = SaasConfig::new(tmp.path());
        saas.node_id = Some(node.into());
        ProfileHost::new(saas, Arc::clone(&operator))
    };
    let (owner, inspector, third) = (
        make_host("node-a"),
        make_host("node-b"),
        make_host("node-c"),
    );
    let id = ProfileId::for_user(
        &format!("doctor-held-{}", uuid::Uuid::new_v4().simple()),
        crate::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    owner.provision(&id).await.unwrap();
    let opened = owner.open(&id).await.unwrap();

    assert_eq!(
        inspector.inspect_for_doctor(&id).await,
        DoctorProfileState::HeldElsewhere
    );
    assert_eq!(inspector.open_count(), 0);

    match third.open(&id).await {
        Err(OpenError::HeldElsewhere(record)) => assert_eq!(record.owner, "node-a"),
        other => panic!("inspection must leave the existing lease with node-a; got {other:?}"),
    }

    drop(opened);
}

fn host(tmp: &tempfile::TempDir, max_open: usize, idle_secs: u64) -> ProfileHost {
    let mut saas = SaasConfig::new(tmp.path());
    saas.max_profiles_open = max_open;
    saas.idle_evict_secs = idle_secs;
    let backend_path = tmp.path().join("profile-storage");
    let backend_url = format!("sqlite:{}", backend_path.display());
    let backend = crate::storage::block_on_anyhow(async move {
        crate::storage::open(&backend_url)
            .await
            .map_err(anyhow::Error::from)
    })
    .expect("profile test SQLite backend opens");
    ProfileHost::with_backend(
        saas,
        CoreContext::for_test(DomainSet::full(), None),
        Some(backend),
    )
    .expect("profile test host uses conditional SQLite leases")
}

fn profile(name: &str) -> ProfileId {
    ProfileId::for_user(name, crate::profiles::ProfileIdMode::Raw).unwrap()
}

fn raw_profile_agent_add() -> tinyflows_bus::schedule::StoreOperation {
    tinyflows_bus::schedule::StoreOperation::AddAgent {
        spec: tinyflows_bus::schedule::AgentJobSpec::new(
            tinyflows_bus::schedule::Schedule::Every { every_ms: 600_000 },
            "profile quota transition probe",
        ),
    }
}

struct RestoreInstalledStorage(Option<Arc<dyn crate::storage::StorageBackend>>);

impl Drop for RestoreInstalledStorage {
    fn drop(&mut self) {
        crate::storage::clear();
        if let Some(previous) = self.0.take() {
            crate::storage::install(previous);
        }
    }
}

struct RetireProfileFence {
    registry: Arc<crate::storage::fence::FenceRegistry>,
    profile_id: String,
    epoch: u64,
}

impl Drop for RetireProfileFence {
    fn drop(&mut self) {
        self.registry.retire(&self.profile_id, self.epoch);
    }
}

#[tokio::test]
async fn provisioning_creates_the_layout_once() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 4, 60);
    let id = profile("alice");
    assert!(host.provision(&id).await.unwrap());
    assert!(
        !host.provision(&id).await.unwrap(),
        "second provision is a no-op"
    );
    let layout = ProfileLayout::new(tmp.path(), &id);
    assert!(layout.workspace_dir.is_dir() && layout.sandbox_dir.is_dir());
    let summary = host.summary(&id).await.unwrap().unwrap();
    assert_eq!(summary.profile_id, id);
    assert!(!summary.open);
}

#[tokio::test]
async fn only_provisioned_profiles_open() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 4, 60);
    let err = host.open(&profile("nobody")).await.unwrap_err();
    assert_eq!(err, OpenError::NotProvisioned(profile("nobody")));
}

#[tokio::test]
async fn an_open_profile_runs_under_its_own_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 4, 60);
    let (a, b) = (profile("alice"), profile("bob"));
    host.provision(&a).await.unwrap();
    host.provision(&b).await.unwrap();
    let state_a = host.open(&a).await.unwrap();
    let state_b = host.open(&b).await.unwrap();

    // A profile's default agent runs as the desktop's does: no agent id, the
    // profile is the tenant key.
    assert_eq!(state_a.context().session_agent(), None);
    assert_eq!(state_b.context().session_agent(), None);
    assert_eq!(state_a.context().profile(), Some(a.as_str()));
    assert_eq!(state_b.context().profile(), Some(b.as_str()));
    let config_a = state_a.context().embedder_config().unwrap();
    let config_b = state_b.context().embedder_config().unwrap();
    assert_ne!(config_a.workspace_dir, config_b.workspace_dir);
    assert!(config_a.workspace_dir.starts_with(tmp.path()));
    assert_eq!(state_a.context().domains(), user_domains());
    assert!(!state_a.context().user_skill_roots());

    // Re-opening hands back the same state.
    assert!(Arc::ptr_eq(&state_a, &host.open(&a).await.unwrap()));
    assert_eq!(host.open_count(), 2);
}

#[tokio::test]
async fn a_full_host_evicts_the_least_recently_used_idle_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 2, 3600);
    let ids: Vec<_> = ["a", "b", "c"].iter().map(|n| profile(n)).collect();
    for id in &ids {
        host.provision(id).await.unwrap();
    }
    drop(host.open(&ids[0]).await.unwrap());
    drop(host.open(&ids[1]).await.unwrap());
    drop(host.open(&ids[2]).await.unwrap());
    assert_eq!(host.open_count(), 2);
    assert!(!host.is_open(&ids[0]), "the oldest idle agent made room");
    assert!(host.is_open(&ids[1]) && host.is_open(&ids[2]));
}

#[tokio::test]
async fn an_profile_in_use_is_never_evicted() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 1, 0);
    let (a, b) = (profile("a"), profile("b"));
    host.provision(&a).await.unwrap();
    host.provision(&b).await.unwrap();
    let held = host.open(&a).await.unwrap();
    host.evict_idle().await;
    assert!(host.is_open(&a), "held agents survive an idle sweep");
    let err = host.open(&b).await.unwrap_err();
    assert!(matches!(err, OpenError::Full { max: 1 }), "{err}");
    drop(held);
    host.open(&b).await.unwrap();
}

#[tokio::test]
async fn idle_profiles_are_swept() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 4, 0);
    let id = profile("a");
    host.provision(&id).await.unwrap();
    drop(host.open(&id).await.unwrap());
    host.evict_idle().await;
    assert_eq!(host.open_count(), 0);
}

#[tokio::test]
async fn deprovisioning_archives_and_closes() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 4, 60);
    let id = profile("a");
    host.provision(&id).await.unwrap();
    drop(host.open(&id).await.unwrap());
    assert!(host.deprovision(&id).await.unwrap());
    assert!(!host.is_open(&id));
    assert!(host.summary(&id).await.unwrap().is_none());
    let archived: Vec<_> = std::fs::read_dir(layout::archive_dir(tmp.path()))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(archived.len(), 1, "state is archived, not deleted");
    assert!(!host.deprovision(&id).await.unwrap());
}

#[tokio::test]
async fn list_reports_provisioned_profiles_only() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 4, 60);
    assert!(host.list().await.unwrap().is_empty());
    let (a, b) = (profile("a"), profile("b"));
    host.provision(&a).await.unwrap();
    host.provision(&b).await.unwrap();
    std::fs::create_dir_all(layout::users_dir(tmp.path()).join("not-an-agent")).unwrap();
    let _held = host.open(&b).await.unwrap();
    let listed = host.list().await.unwrap();
    assert_eq!(listed.len(), 2);
    let open: Vec<_> = listed
        .iter()
        .filter(|s| s.open)
        .map(|s| &s.profile_id)
        .collect();
    assert_eq!(open, vec![&b]);
}

#[tokio::test]
async fn a_reprovisioned_profile_does_not_inherit_the_old_credential() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 4, 60);
    let id = profile(&format!("returning-{}", uuid::Uuid::new_v4()));
    host.provision(&id).await.unwrap();
    let config = host.open(&id).await.unwrap().config.clone();
    super::super::credentials::store(
        &config,
        super::super::credentials::UserCredentialKind::Session,
        "old-session",
        None,
    )
    .await
    .unwrap();
    assert!(host.summary(&id).await.unwrap().unwrap().has_credential);
    host.deprovision(&id).await.unwrap();
    host.provision(&id).await.unwrap();
    assert!(
        !host.summary(&id).await.unwrap().unwrap().has_credential,
        "deprovisioning must forget the credential"
    );
}

#[tokio::test]
async fn a_clean_reopen_does_not_recover_but_a_takeover_does() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 4, 0);
    let previous = crate::storage::install(Arc::clone(
        host.backend.as_ref().expect("test host has shared storage"),
    ));
    let _restore_storage = RestoreInstalledStorage(previous);
    let id = profile(&format!("recover-{}", uuid::Uuid::new_v4()));
    host.provision(&id).await.unwrap();
    let workspace = host.layout_of(&id).workspace_dir;
    let in_flight = |thread: &str| {
        let state = tinyagents_session::turn_state::TurnState::started(
            thread,
            "req",
            4,
            "2026-10-09T00:00:00Z",
        );
        crate::storage::sessions::default_turn_states_for(&workspace)
            .put(&state)
            .unwrap();
    };
    let lifecycle = |thread: &str| {
        crate::storage::sessions::default_turn_states_for(&workspace)
            .get(thread)
            .unwrap()
            .unwrap()
            .lifecycle
    };

    // The first holder ever: nothing to recover.
    drop(host.open(&id).await.unwrap());
    in_flight("t1");
    // An eviction releases the lease cleanly; re-opening must not mark a
    // turn this process may still own as interrupted.
    host.evict_idle().await;
    drop(host.open(&id).await.unwrap());
    assert_eq!(
        lifecycle("t1"),
        tinyagents_session::turn_state::TurnLifecycle::Started
    );

    // A second host on the same root (a restarted process) finds the lease
    // never released once the first is gone. Expire the persisted lease
    // explicitly so takeover does not depend on waiting for the TTL.
    let holder = host;
    let second = self::host(&tmp, 4, 0);
    let refused = second.open(&id).await.unwrap_err();
    assert!(matches!(refused, OpenError::HeldElsewhere(_)), "{refused}");
    drop(holder);

    let backend = second
        .backend
        .as_ref()
        .expect("test host has shared storage");
    let cluster = backend
        .for_scope(&crate::storage::Scope::new(crate::storage::lease::CLUSTER_SCOPE).unwrap())
        .unwrap();
    let lease_id = tinystoragedrivers::Key::new(format!(
        "{}/{}",
        crate::storage::lease::LEASE_OWNER,
        id.as_str()
    ))
    .unwrap();
    let lease = cluster
        .documents()
        .get(crate::storage::lease::LEASE_COLLECTION, lease_id.as_str())
        .await
        .unwrap()
        .expect("the crashed holder's lease remains recorded");
    let mut expired = lease.doc.clone();
    expired["expires_ms"] = 0.into();
    cluster
        .documents()
        .put(
            crate::storage::lease::LEASE_COLLECTION,
            lease_id.as_str(),
            expired,
            tinystoragedrivers::Precondition::Version(lease.version),
        )
        .await
        .unwrap();

    drop(second.open(&id).await.unwrap());
    assert_eq!(
        lifecycle("t1"),
        tinyagents_session::turn_state::TurnLifecycle::Interrupted
    );
}

#[tokio::test]
async fn recovery_of_an_empty_workspace_is_a_no_op() {
    let tmp = tempfile::tempdir().unwrap();
    let id = profile("empty");
    recover_workspace(&id, tmp.path());
}

#[tokio::test]
async fn an_profile_in_use_is_not_archived_from_under_it() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 4, 60);
    let id = profile("alice");
    host.provision(&id).await.unwrap();
    let state = host.open(&id).await.unwrap();
    let err = host.deprovision(&id).await.unwrap_err();
    assert!(err.contains("in use"), "{err}");
    assert!(host.is_open(&id), "a refused deprovision leaves it open");
    drop(state);
    assert!(host.deprovision(&id).await.unwrap());
    assert!(!host.is_open(&id));
}

#[tokio::test]
async fn deprovisioning_twice_in_a_second_archives_twice() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 4, 60);
    let id = profile("alice");
    for _ in 0..2 {
        host.provision(&id).await.unwrap();
        assert!(host.deprovision(&id).await.unwrap());
    }
    let archived = std::fs::read_dir(layout::archive_dir(tmp.path()))
        .unwrap()
        .count();
    assert_eq!(archived, 2);
}

#[tokio::test]
async fn opening_an_open_profile_sweeps_the_idle_ones() {
    let tmp = tempfile::tempdir().unwrap();
    // Everything is idle the moment it is released.
    let host = host(&tmp, 4, 0);
    let (a, b) = (profile("alice"), profile("bob"));
    host.provision(&a).await.unwrap();
    host.provision(&b).await.unwrap();
    drop(host.open(&a).await.unwrap());
    let held = host.open(&b).await.unwrap();
    // Re-opening bob (already open) closes idle alice.
    let again = host.open(&b).await.unwrap();
    assert!(!host.is_open(&a), "alice was idle and is closed");
    assert!(host.is_open(&b), "bob is in use and stays");
    drop((held, again));
}

#[tokio::test]
async fn one_unreadable_profile_does_not_hide_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let host = ProfileHost::new(
        SaasConfig::new(tmp.path()),
        CoreContext::for_test(DomainSet::full(), None),
    );
    let (a, b) = (profile("alice"), profile("bob"));
    host.provision(&a).await.unwrap();
    host.provision(&b).await.unwrap();
    std::fs::write(ProfileLayout::new(tmp.path(), &a).meta_path, "not toml = [").unwrap();
    let listed = host.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].profile_id, b);
}

#[tokio::test]
async fn a_provisioned_config_needs_no_profile_slot() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 1, 60);
    let (a, b) = (profile("alice"), profile("bob"));
    host.provision(&a).await.unwrap();
    host.provision(&b).await.unwrap();
    let _held = host.open(&a).await.unwrap();
    assert!(host.open(&b).await.is_err(), "the only slot is taken");
    assert!(host.provisioned_config(&b).await.is_ok());
    assert!(host.provisioned_config(&profile("nobody")).await.is_err());
}

#[tokio::test]
async fn an_open_profile_is_gated_by_its_own_policy_not_the_operators() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 4, 60);
    let id = profile("alice");
    host.provision(&id).await.unwrap();
    let state = host.open(&id).await.unwrap();
    assert_eq!(state.context().profile(), Some(id.as_str()));

    let own = state
        .context()
        .agent_policy()
        .expect("the profile carries a policy");
    assert!(own.enabled, "the forced autonomy policy is on");
    assert!(own.workspace_only);
    let effective = CoreContext::scope(Arc::clone(state.context()), async {
        crate::security::live_policy::effective_in(&crate::core::runtime::DispatchCtx::ambient())
    })
    .await
    .expect("a policy is in effect");
    assert!(
        Arc::ptr_eq(&effective, &own),
        "inside the profile's scope the gate answers with its policy"
    );
}

#[tokio::test]
async fn a_profile_with_a_live_turn_is_not_evicted() {
    let tmp = tempfile::tempdir().unwrap();
    let host = host(&tmp, 1, 0);
    let (a, b) = (profile("a"), profile("b"));
    host.provision(&a).await.unwrap();
    host.provision(&b).await.unwrap();
    // The request that started the turn has answered and let go of the
    // state; the turn keeps running on a context derived from the profile's.
    let context = Arc::clone(host.open(&a).await.unwrap().context());
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let turn = tokio::spawn(CoreContext::scope_with_turn_origin(context, None, async {
        let _ = released.await;
    }));
    tokio::task::yield_now().await;

    host.evict_idle().await;
    assert!(host.is_open(&a), "a live turn keeps its profile open");
    let err = host.open(&b).await.unwrap_err();
    assert!(matches!(err, OpenError::Full { max: 1 }), "{err}");
    assert!(host.deprovision(&a).await.unwrap_err().contains("in use"));

    release.send(()).unwrap();
    turn.await.unwrap();
    host.evict_idle().await;
    assert!(!host.is_open(&a), "evicted once the turn ended");
    host.open(&b).await.unwrap();
}

#[tokio::test]
async fn the_operators_iteration_cap_binds_every_profile_whatever_its_config_says() {
    let tmp = tempfile::tempdir().unwrap();
    let mut saas = SaasConfig::new(tmp.path());
    saas.max_tool_iterations = Some(4);
    let host = ProfileHost::new(saas, CoreContext::for_test(DomainSet::full(), None));
    let id = profile("alice");
    host.provision(&id).await.unwrap();
    let layout = ProfileLayout::new(tmp.path(), &id);
    std::fs::write(
        &layout.config_path,
        "[agent]\nmax_tool_iterations_override = 99\n",
    )
    .unwrap();
    let forced = host.provisioned_config(&id).await.unwrap();
    let opened = host.open(&id).await.unwrap();
    assert_eq!(forced.agent.max_tool_iterations_override, Some(4));
    assert_eq!(opened.config.agent.max_tool_iterations_override, Some(4));
}

#[path = "host_migration_fence_tests.rs"]
mod migration_fence_tests;

#[path = "host_fence_tests.rs"]
mod fence_tests;

#[path = "host_cron_cap_tests.rs"]
mod cron_cap_tests;
