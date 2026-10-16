use super::*;
use crate::memory::test_fixtures::config_in;
use crate::security::credentials::api_key::store_api_key;

use crate::security::credentials::{AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};

fn off_reason(binding: Binding) -> String {
    match binding {
        Binding::Off { reason, .. } => reason,
        Binding::On(bound) => panic!("expected off, bound {}", bound.id),
    }
}

#[test]
fn tinyhumans_with_nothing_registered_is_unavailable() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.engine = TINYHUMANS_ENGINE.to_string();
    let binding = resolve(&config);
    assert!(!binding.is_on());
    assert!(!is_on(&config));
    match binding {
        Binding::Off {
            engine, endpoint, ..
        } => {
            assert_eq!(engine.as_deref(), Some(TINYHUMANS_ENGINE));
            assert_eq!(endpoint, None);
        }
        Binding::On(_) => unreachable!(),
    }
    let error = resolve(&config).engine().unwrap_err();
    assert_eq!(error.code(), super::super::error::MEMORY_OFF);
}

#[test]
fn a_local_session_token_is_not_a_tinyhumans_credential() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    AuthService::from_config(&config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "desktop.test.local",
            std::collections::HashMap::new(),
            true,
        )
        .unwrap();
    assert!(!has_key(&config, TINYHUMANS_ENGINE));
}

#[test]
fn unknown_and_blank_engine_ids_are_off() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.engine = "nope".to_string();
    assert!(off_reason(resolve(&config)).contains("not available"));
    config.memory.engine = "  ".to_string();
    assert!(off_reason(resolve(&config)).contains("no memory engine"));
    assert!(!has_key(&config, "nope"));
}

#[test]
fn cortexdb_is_off_until_a_key_is_stored_and_rebuilds_on_a_new_key() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.engine = CORTEXDB_ENGINE.to_string();
    let reason = off_reason(resolve(&config));
    assert!(reason.contains("CortexDB API key"), "{reason}");
    assert!(!has_key(&config, CORTEXDB_ENGINE));

    store_cortexdb_key(&config, "  cdb-key-one  ").unwrap();
    assert_eq!(
        read_cortexdb_key(&config).unwrap().as_deref(),
        Some("cdb-key-one")
    );
    assert!(has_key(&config, CORTEXDB_ENGINE));
    let first = resolve(&config).engine().expect("bound");
    assert_eq!(first.id, CORTEXDB_ENGINE);
    assert_eq!(
        first.endpoint,
        tinymemory_integrations::cortex::CORTEX_API_ENDPOINT
    );

    store_cortexdb_key(&config, "cdb-key-two").unwrap();
    let second = resolve(&config).engine().expect("rebound");
    assert!(
        !Arc::ptr_eq(&first.engine, &second.engine),
        "a new key builds a new engine"
    );

    assert!(clear_cortexdb_key(&config).unwrap());
    assert!(!is_on(&config));
}

#[test]
fn switching_observed_actor_rebuilds_the_cortexdb_engine() {
    let _stable = CACHE_STABLE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.engine = CORTEXDB_ENGINE.to_string();
    store_cortexdb_key(&config, "cdb-key-actor").unwrap();
    let off = resolve(&config).engine().expect("bound");
    assert!(
        Arc::ptr_eq(
            &off.engine,
            &resolve(&config).engine().expect("bound").engine
        ),
        "unchanged config reuses the engine"
    );

    config.memory.observed_actor = true;
    let on = resolve(&config).engine().expect("rebound");
    assert!(
        !Arc::ptr_eq(&off.engine, &on.engine),
        "the setting is part of the cache fingerprint"
    );
}

#[test]
fn a_blank_cortexdb_key_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let error = store_cortexdb_key(&config, "   ").unwrap_err();
    assert_eq!(error.code(), super::super::error::INVALID_REQUEST);
}

#[test]
fn the_cache_fingerprint_never_contains_the_key() {
    let digest = key_digest("super-secret-key");
    assert_eq!(digest.len(), 16);
    assert!(!digest.contains("secret"));
    assert_eq!(digest, key_digest("super-secret-key"));
    assert_ne!(digest, key_digest("another-key"));
}

#[test]
fn an_installed_test_engine_wins_and_is_reported_on() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert!(!is_on(&config));
    crate::memory::test_fixtures::bind_reference(&config);
    let bound = resolve(&config).engine().unwrap();
    assert_eq!(bound.id, "reference");
    assert!(format!("{bound:?}").contains("reference"));
}

/// A config living where a signed-in user's does: `<tmp>/users/<id>/`.
fn user_config(tmp: &tempfile::TempDir, user: &str) -> Config {
    let dir = tmp.path().join("users").join(user);
    std::fs::create_dir_all(&dir).unwrap();
    let mut config = config_in(tmp);
    config.config_path = dir.join("config.toml");
    config
}

#[test]
fn layout_v3_binds_its_own_engine_beside_legacy() {
    let _stable = CACHE_STABLE
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    // A user id of its own: the credential store is shared by id, so another
    // test storing a key for the same id would change the fingerprint.
    let mut config = user_config(&tmp, "6512ab0f6512ab0f6512ab0f");
    config.memory.engine = CORTEXDB_ENGINE.to_string();
    store_cortexdb_key(&config, "cdb-key-layout").unwrap();

    let legacy = resolve(&config).engine().expect("legacy bound");
    config.memory.layout = crate::config::MemoryLayoutMode::V3;
    let v3 = resolve(&config).engine().expect("v3 bound");
    assert!(
        !Arc::ptr_eq(&legacy.engine, &v3.engine),
        "each layout has its own engine"
    );

    // The migration holds both at once, whatever the setting says.
    let explicit_legacy = bind_with_root(&config, None).unwrap();
    let explicit_v3 = bind_with_root(&config, Some("org:6512ab0f6512ab0f6512ab0f")).unwrap();
    assert!(Arc::ptr_eq(&explicit_legacy.engine, &legacy.engine));
    assert!(Arc::ptr_eq(&explicit_v3.engine, &v3.engine));
}

#[test]
fn layout_v3_before_anyone_signs_in_is_off() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = user_config(&tmp, crate::config::PRE_LOGIN_USER_ID);
    config.memory.engine = CORTEXDB_ENGINE.to_string();
    config.memory.layout = crate::config::MemoryLayoutMode::V3;
    let reason = off_reason(resolve(&config));
    assert!(reason.contains("sign in"), "{reason}");
}

#[test]
fn an_installed_engine_has_one_layout_unless_one_is_installed_per_root() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    install_test_engine(
        &config.workspace_dir,
        Arc::new(tinymemory_api::conformance::ReferenceEngine::new()),
    );
    assert!(bind_with_root(&config, Some("user:42")).is_err());
    assert_eq!(
        bind_with_root(&config, None).unwrap().endpoint,
        "test://engine",
        "the legacy layout binds the installed test engine"
    );

    install_test_engine_for_root(
        &config.workspace_dir,
        Some("user:42"),
        Arc::new(tinymemory_api::conformance::ReferenceEngine::new()),
    );
    install_test_engine_for_root(
        &config.workspace_dir,
        None,
        Arc::new(tinymemory_api::conformance::ReferenceEngine::new()),
    );
    let v3 = bind_with_root(&config, Some("user:42")).unwrap();
    let legacy = bind_with_root(&config, None).unwrap();
    assert!(!Arc::ptr_eq(&v3.engine, &legacy.engine));
}

#[test]
fn a_root_sets_the_scope_root_its_owner_and_the_cache_key() {
    // CortexDB's own API: `org:<id>` is the root, owned by the actor.
    let (settings, key) = rooted(EngineSettings::default(), Some("org:42"), false, false);
    assert_eq!(settings.scope_root.as_deref(), Some("org:42"));
    assert_eq!(settings.scope_owner.as_deref(), Some("user:42"));
    assert!(!settings.tenant_root);
    assert_eq!(settings.retired_scope_root, None);
    assert_eq!(key, "|root=org:42|tenant=false|retired=-");
    let (settings, key) = rooted(EngineSettings::default(), None, false, true);
    assert_eq!(settings.scope_root, None);
    assert_eq!(settings.retired_scope_root, None);
    assert_eq!(key, "|root=legacy");
}

#[test]
fn the_hosted_wire_sends_no_root_and_legacy_read_names_the_user_segment() {
    // TinyHumans: the backend pins `org:<id>`; no root, no `user:` segment.
    let (settings, key) = rooted(EngineSettings::default(), Some("org:42"), true, false);
    assert!(settings.tenant_root);
    assert_eq!(settings.scope_root, None);
    assert_eq!(settings.scope_owner, None);
    assert_eq!(settings.retired_scope_root, None);
    // During the move, the earlier `user:<id>` root is read and forgotten.
    let (settings, read_too) = rooted(EngineSettings::default(), Some("org:42"), true, true);
    assert_eq!(settings.retired_scope_root.as_deref(), Some("user:42"));
    assert_ne!(key, read_too, "the flag rebuilds the engine");
    let (settings, _) = rooted(EngineSettings::default(), Some("org:42"), false, true);
    assert_eq!(settings.retired_scope_root.as_deref(), Some("user:42"));
}

#[test]
fn legacy_user_segment_read_is_on_by_default_and_written_only_when_off() {
    let config = crate::config::MemoryConfig::default();
    assert!(config.legacy_user_segment_read);
    let on = toml::to_string(&config).unwrap();
    assert!(!on.contains("legacy_user_segment_read"), "{on}");
    let off = crate::config::MemoryConfig {
        legacy_user_segment_read: false,
        ..config
    };
    let written = toml::to_string(&off).unwrap();
    assert!(
        written.contains("legacy_user_segment_read = false"),
        "{written}"
    );
    let read: crate::config::MemoryConfig = toml::from_str("").unwrap();
    assert!(read.legacy_user_segment_read);
}

#[tokio::test]
async fn switching_to_v3_persists_and_rebinds_the_persons_own_config() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = user_config(&tmp, "6512ab0f6512ab0f6512ab10");
    config.memory.engine = CORTEXDB_ENGINE.to_string();
    config.save().await.unwrap();
    store_cortexdb_key(&config, "cdb-key-switch").unwrap();
    let legacy = resolve(&config).engine().expect("legacy bound");

    super::super::scope::switch_to_v3(&config).await.unwrap();

    let saved = Config::load_from_config_path(&config.config_path, &config.workspace_dir)
        .await
        .unwrap();
    assert!(
        super::super::scope::layout_is_v3(&saved),
        "v3 written to this person's file"
    );
    let v3 = resolve(&saved).engine().expect("v3 bound");
    assert!(
        !Arc::ptr_eq(&legacy.engine, &v3.engine),
        "the switch rebinds"
    );
    assert!(
        !super::super::scope::layout_is_v3(&config),
        "the caller's copy is not the source of truth"
    );
}

#[test]
fn saas_never_admits_the_host_engine() {
    assert!(!admits_host_engine_with(true));
    assert!(admits_host_engine_with(false));
}

#[test]
fn host_binding_is_ignored_in_saas() {
    assert_eq!(host_binding_with(true, Some(1), None).unwrap(), None);
    // The scope-root refusal never fires in SaaS: the host engine is not
    // in play at all.
    assert_eq!(
        host_binding_with(true, Some(1), Some("user:a")).unwrap(),
        None
    );
}

#[test]
fn host_binding_is_used_without_a_scope_root_outside_saas() {
    assert_eq!(host_binding_with(false, Some(1), None).unwrap(), Some(1));
    assert_eq!(host_binding_with::<u8>(false, None, None).unwrap(), None);
}

#[test]
fn host_binding_refuses_a_scope_root_outside_saas() {
    assert!(host_binding_with(false, Some(1), Some("user:a")).is_err());
    assert_eq!(
        host_binding_with::<u8>(false, None, Some("user:a")).unwrap(),
        None
    );
}

#[test]
fn a_disabled_memory_component_binds_no_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.components.memory = false;
    assert!(off_reason(resolve(&config)).contains("components"));
    assert!(!is_on(&config));
    assert!(switched_off(&config));
    assert!(matches!(
        bind_with_root(&config, None),
        Err(MemoryError::Off(_))
    ));
}

#[test]
fn a_disabled_component_overrides_an_installed_test_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    assert!(!switched_off(&config));
    config.memory.engine = DISABLED_ENGINE.to_string();
    assert!(switched_off(&config));
}
