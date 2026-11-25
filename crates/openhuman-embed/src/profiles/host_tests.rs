//! Immutable profile host configuration rejects broader capability requests.

use super::*;

fn unbound_ctx() -> openhuman_core::core::runtime::DispatchCtx {
    openhuman_core::core::runtime::DispatchCtx::unbound()
}

#[test]
fn wildcard_profile_scope_is_refused_before_boot() {
    let config = SaasConfig::new("/srv/openhuman");
    let options = ProfileHostOptions {
        scope: Some(ToolScopeSpec::Wildcard),
        ..Default::default()
    };
    assert!(
        matches!(options.finish(&config), Err(ProfileError::Boot(message)) if message.contains("named or host-only"))
    );
}

#[test]
fn destructive_channel_ceiling_is_refused_before_boot() {
    let config = SaasConfig::new("/srv/openhuman");
    let options = ProfileHostOptions {
        channels: HashMap::from([("telegram".into(), PermissionLevel::Dangerous)]),
        ..Default::default()
    };
    assert!(
        matches!(options.finish(&config), Err(ProfileError::Boot(message)) if message.contains("dangerous"))
    );
}

#[test]
fn named_selection_never_overrides_hard_denies_or_default_readonly() {
    let config = SaasConfig::new("/srv/openhuman");
    let options = ProfileHostOptions {
        scope: Some(ToolScopeSpec::Named(vec![
            "memory".into(),
            "shell".into(),
            "spawn_subagent".into(),
            "curl".into(),
            "run_workflow".into(),
        ])),
        ..Default::default()
    };
    let capabilities = options.finish(&config).unwrap();
    let unbound = unbound_ctx();
    assert!(capabilities.admits(&unbound, "memory"));
    for name in ["shell", "spawn_subagent", "curl", "run_workflow", "hidden"] {
        assert!(!capabilities.admits(&unbound, name), "{name}");
    }
    assert_eq!(
        capabilities.channel_permission("telegram"),
        PermissionLevel::ReadOnly
    );
    assert!(!capabilities.authorizes_approval(&unbound, "memory"));
}

#[test]
fn profiles_keep_a_nonempty_readonly_permission_map_without_operator_grants() {
    let saas = SaasConfig::new("/srv/openhuman");
    let capabilities = ProfileHostOptions::default().finish(&saas).unwrap();
    let id = super::super::ProfileId::parse("alice").unwrap();
    let layout = openhuman_core::profiles::layout::profile_layout(&saas.root, &id);
    let mut config = openhuman_core::profiles::layout::profile_config(&layout, &id);
    capabilities.configure(&mut config);
    assert!(!config.agent.channel_permissions.is_empty());
    assert!(config
        .agent
        .channel_permissions
        .values()
        .all(|level| level == "readonly"));
}

#[test]
fn host_only_admission_does_not_authorize_supervised_mutation_or_unknown_channels() {
    fn allow(_: &crate::seams::ToolHookContext) -> crate::permission::PermissionFuture<'_> {
        Box::pin(async { crate::seams::ToolHookDecision::default() })
    }
    let config = SaasConfig::new("/srv/openhuman");
    let options = ProfileHostOptions {
        scope: Some(ToolScopeSpec::HostOnly),
        permission: Some(Arc::new(crate::permission::PermissionHook::new(allow))),
        channels: HashMap::from([("telegram".into(), PermissionLevel::Execute)]),
        ..Default::default()
    };
    let capabilities = options.finish(&config).unwrap();
    let unbound = unbound_ctx();
    assert!(capabilities.admits(&unbound, "custom_host_mutation"));
    assert!(!capabilities.authorizes_approval(&unbound, "custom_host_mutation"));
    assert_eq!(
        capabilities.channel_permission("telegram"),
        PermissionLevel::Execute
    );
    assert_eq!(
        capabilities.channel_permission("unknown_channel"),
        PermissionLevel::ReadOnly
    );
}
