use super::*;

#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn profile_context_conflict_refresh_keeps_host_owned_restrictions() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = memory_backend();
    let _forced = ForcedDocumentSource::new(
        scoped(storage.as_ref(), "tenant-profile-refresh"),
        Scope::new("tenant-profile-refresh").unwrap(),
        keys(16),
    );
    let operator = CoreContext::for_test(DomainSet::full(), None);
    let saas = crate::core::runtime::SaasConfig::new(tmp.path());
    // A storage-backed lease, so the profile fence carries an atomic write
    // condition: the fenced backend refuses unguarded (file-lease) handles.
    let fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend: Arc<dyn tinystoragedrivers::StorageBackend> = Arc::clone(&fixture.backend);
    let host = crate::profiles::ProfileHost::with_backend(saas, operator, Some(backend))
        .unwrap()
        .with_fences(Arc::new(crate::storage::fence::FenceRegistry::new(
            crate::profiles::lease::now_ms,
        )));
    let id = crate::profiles::ProfileId::for_user(
        "profile-refresh",
        crate::profiles::ProfileIdMode::Raw,
    )
    .unwrap();
    host.provision(&id).await.unwrap();
    let profile = host.open(&id).await.unwrap();
    let context = Arc::clone(profile.context());

    let initial = CoreContext::scope(context.clone(), async {
        DispatchCtx::new(context.clone())
            .load_config()
            .await
            .unwrap()
    })
    .await;
    assert!(initial.autonomy.enabled);
    assert!(initial.autonomy.workspace_only);
    assert_eq!(initial.memory.agent_id.as_deref(), Some(id.as_str()));
    assert_eq!(
        initial.memory.layout,
        crate::config::schema::MemoryLayoutMode::Legacy
    );
    assert_eq!(initial.agent.max_tool_iterations_override, None);
    CoreContext::scope(context.clone(), initial.save())
        .await
        .unwrap();

    let mut stale = CoreContext::scope(context.clone(), async {
        DispatchCtx::new(context.clone())
            .load_config()
            .await
            .unwrap()
    })
    .await;
    let mut external = Config::load_from_config_path(&stale.config_path, &stale.workspace_dir)
        .await
        .unwrap();
    external.default_model = Some("external-winner".into());
    external.autonomy.enabled = false;
    external.autonomy.workspace_only = false;
    external.memory.agent_id = Some("external-agent".into());
    external.memory.root = Some("team:external".into());
    external.memory.layout = crate::config::schema::MemoryLayoutMode::V3;
    external.agent.max_tool_iterations_override = Some(99);
    external.agent.channel_permissions.clear();
    external.save().await.unwrap();

    stale.default_temperature = 0.8;
    let error = CoreContext::scope(context.clone(), stale.save())
        .await
        .expect_err("the stale profile snapshot must conflict with the external write");
    assert!(error.chain().any(|cause| {
        cause
            .downcast_ref::<tinystoragedrivers::StorageError>()
            .is_some_and(|error| error.kind() == tinystoragedrivers::ErrorKind::Conflict)
    }));

    let refreshed = CoreContext::scope(context.clone(), async {
        DispatchCtx::new(context.clone())
            .load_config()
            .await
            .unwrap()
    })
    .await;
    assert_eq!(refreshed.default_model.as_deref(), Some("external-winner"));
    assert!(refreshed.autonomy.enabled);
    assert!(refreshed.autonomy.workspace_only);
    assert_eq!(refreshed.memory.agent_id.as_deref(), Some(id.as_str()));
    assert_eq!(
        refreshed.memory.layout,
        crate::config::schema::MemoryLayoutMode::Legacy
    );
    assert_eq!(refreshed.agent.max_tool_iterations_override, None);
    assert!(!refreshed.agent.channel_permissions.is_empty());
    let nested = context.derive_with(ContextOverlay::new(
        refreshed,
        context.domains(),
        crate::tools::toolpacks::ToolGroups::none(),
    ));
    let mut nested_stale = CoreContext::scope(nested.clone(), async {
        DispatchCtx::new(nested.clone())
            .load_config()
            .await
            .unwrap()
    })
    .await;
    let mut later_external =
        Config::load_from_config_path(&nested_stale.config_path, &nested_stale.workspace_dir)
            .await
            .unwrap();
    later_external.default_model = Some("external-after-nested".into());
    later_external.autonomy.enabled = false;
    later_external.memory.agent_id = Some("later-external-agent".into());
    later_external.agent.channel_permissions.clear();
    later_external.save().await.unwrap();

    nested_stale.default_temperature = 0.9;
    CoreContext::scope(nested.clone(), nested_stale.save())
        .await
        .expect_err("a nested profile snapshot must conflict with the later external write");
    let nested_refreshed = CoreContext::scope(nested.clone(), async {
        DispatchCtx::new(nested.clone())
            .load_config()
            .await
            .unwrap()
    })
    .await;
    assert_eq!(
        nested_refreshed.default_model.as_deref(),
        Some("external-after-nested")
    );
    assert!(nested_refreshed.autonomy.enabled);
    assert_eq!(
        nested_refreshed.memory.agent_id.as_deref(),
        Some(id.as_str())
    );
    assert!(!nested_refreshed.agent.channel_permissions.is_empty());
}
