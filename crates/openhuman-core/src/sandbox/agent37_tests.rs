//! The agent37 backend: what it refuses before anything leaves the host, and
//! how `[runtime.agent37]` becomes its boxes' settings and retention. The box
//! lifecycle itself is the shared registry's, tested in `boxes`.

use super::*;
use crate::sandbox::boxes::{
    BoxError, BoxSettings, OwnerScope, RegistryLimits, RetainPolicy, AGENT37_WORKDIR,
};

fn local_only() -> Config {
    let mut config = Config::default();
    config.privacy.mode = crate::config::PrivacyMode::LocalOnly;
    config
}

#[tokio::test]
async fn local_only_privacy_refuses_a_command_before_it_leaves_the_host() {
    let error = execute(
        &DispatchCtx::unbound(),
        &local_only(),
        "id",
        &HashMap::new(),
        Duration::from_secs(1),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error.downcast_ref::<BoxError>(),
        Some(&BoxError::LocalOnly),
        "{error}"
    );
    assert!(error.to_string().contains("local_only"));
}

#[test]
fn only_local_only_privacy_forbids_agent37() {
    let mut config = Config::default();
    // Standard (the default) allows agent37's off-device execution.
    assert_eq!(BoxBackend::Agent37.check_allowed(&config), Ok(()));
    // Sensitive is not local-only, so it still allows agent37.
    config.privacy.mode = crate::config::PrivacyMode::Sensitive;
    assert_eq!(BoxBackend::Agent37.check_allowed(&config), Ok(()));
    // OpenShell runs on the operator's own gateway.
    assert_eq!(BoxBackend::OpenShell.check_allowed(&local_only()), Ok(()));
}

#[test]
fn box_settings_come_from_the_agent37_runtime_config() {
    let mut config = Config::default();
    config.runtime.docker.cpu_limit = Some(8.0);
    config.runtime.docker.memory_limit_mb = Some(1);
    config.runtime.agent37.template = "team-sandbox".into();
    config.runtime.agent37.cpu_millis = 4_000;
    config.runtime.agent37.memory_mb = 16_384;

    let settings = BoxBackend::Agent37.settings(&config).unwrap();

    assert_eq!(
        settings,
        BoxSettings {
            image: "team-sandbox".into(),
            cpu_millis: 4_000,
            memory_bytes: 16_384 * 1024 * 1024,
            workdir: AGENT37_WORKDIR.into(),
            owner_scope: BoxBackend::Agent37.owner_scope(&config),
        }
    );
    assert_eq!(BoxBackend::Agent37.workdir(), "/home/node");
}

#[test]
fn box_settings_ask_for_the_maximum_when_memory_overflows() {
    let mut config = Config::default();
    config.runtime.agent37.memory_mb = u64::MAX;

    let settings = BoxBackend::Agent37.settings(&config).unwrap();
    assert_eq!(settings.memory_bytes, u64::MAX);
}

#[test]
fn a_blank_template_is_refused_with_a_config_error() {
    let mut config = Config::default();
    config.runtime.agent37.template = "  ".into();

    let refused = BoxBackend::Agent37.settings(&config).unwrap_err();
    assert!(matches!(refused, BoxError::InvalidConfig(_)), "{refused:?}");
    assert!(refused.to_string().contains("[runtime.agent37]"));
}

#[test]
fn boxes_are_retained_with_the_configured_sleep_and_expiry() {
    let mut config = Config::default();
    config.runtime.agent37.idle_sleep_secs = 1_200;
    config.runtime.agent37.delete_after_secs = 86_400;
    config.runtime.agent37.max_live = 7;

    assert_eq!(
        BoxBackend::Agent37.limits(&config),
        RegistryLimits {
            idle_ttl: Duration::from_secs(1_200),
            max_live: 7,
            retain: Some(RetainPolicy {
                idle_sleep_secs: 1_200,
                delete_after_secs: 86_400,
            }),
        }
    );
}

#[test]
fn the_sleep_delay_is_clamped_to_what_agent37_accepts() {
    let mut config = Config::default();
    for (configured, used) in [
        (0, 300),
        (299, 300),
        (300, 300),
        (86_400, 86_400),
        (u32::MAX, 86_400),
    ] {
        config.runtime.agent37.idle_sleep_secs = configured;
        let limits = BoxBackend::Agent37.limits(&config);
        assert_eq!(limits.retain.unwrap().idle_sleep_secs, used, "{configured}");
        assert_eq!(limits.idle_ttl, Duration::from_secs(u64::from(used)));
    }
}

#[test]
fn zero_expiry_and_zero_capacity_are_raised_to_one() {
    let mut config = Config::default();
    config.runtime.agent37.delete_after_secs = 0;
    config.runtime.agent37.max_live = 0;

    let limits = BoxBackend::Agent37.limits(&config);
    assert_eq!(limits.retain.unwrap().delete_after_secs, 1);
    assert_eq!(limits.max_live, 1);
}

#[test]
fn openshell_boxes_are_never_retained() {
    assert_eq!(
        BoxBackend::OpenShell.limits(&Config::default()).retain,
        None
    );
    assert_eq!(BoxBackend::OpenShell.workdir(), "/sandbox");
}

#[test]
fn the_owner_scope_follows_the_workspace_and_the_retention_namespace() {
    let mut first_user = Config::default();
    first_user.workspace_dir = "/home/a/.openhuman/users/u1/workspace".into();
    let mut next_user = first_user.clone();
    next_user.workspace_dir = "/home/a/.openhuman/users/u2/workspace".into();
    let mut staging = first_user.clone();
    staging.runtime.agent37.retention_namespace = "staging".into();
    let mut prod = first_user.clone();
    prod.runtime.agent37.retention_namespace = "prod".into();
    let mut padded = prod.clone();
    padded.runtime.agent37.retention_namespace = "  prod ".into();

    let agent37 = |config: &Config| BoxBackend::Agent37.owner_scope(config);
    assert_ne!(agent37(&first_user), agent37(&next_user));
    assert_ne!(agent37(&staging), agent37(&prod));
    assert_ne!(agent37(&first_user), agent37(&prod));
    assert_eq!(agent37(&prod), agent37(&padded));
    assert_ne!(agent37(&prod), OwnerScope::default());
    // OpenShell boxes are never retained; the workspace alone scopes them.
    assert_eq!(
        BoxBackend::OpenShell.owner_scope(&staging),
        BoxBackend::OpenShell.owner_scope(&prod)
    );
    assert_ne!(
        BoxBackend::OpenShell.owner_scope(&first_user),
        BoxBackend::OpenShell.owner_scope(&next_user)
    );
    assert_eq!(
        BoxBackend::Agent37.settings(&prod).unwrap().owner_scope,
        agent37(&prod)
    );
}
