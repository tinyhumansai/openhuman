use super::*;

fn config_with(url: Option<&str>) -> Config {
    let mut config = Config::default();
    config.storage.url = url.map(str::to_string);
    config
}

#[test]
fn the_environment_wins_over_the_config_file() {
    let config = config_with(Some("sqlite:/from/config"));
    assert_eq!(
        url_from(Some("memory".into()), &config).as_deref(),
        Some("memory")
    );
    assert_eq!(
        url_from(None, &config).as_deref(),
        Some("sqlite:/from/config")
    );
}

#[test]
fn blank_values_fall_through_to_the_workspace_default() {
    let config = config_with(None);
    let default = config::default_url(&config.workspace_dir);
    assert_eq!(url_from(None, &config).as_deref(), Some(default.as_str()));
    assert_eq!(
        url_from(Some("  ".into()), &config_with(Some(""))).as_deref(),
        Some(default.as_str())
    );
    assert_eq!(
        url_from(Some(" ".into()), &config_with(Some(" memory "))).as_deref(),
        Some("memory"),
        "a blank override falls through to the file"
    );
}

#[test]
fn the_default_url_is_a_directory_under_the_workspace() {
    let config = config_with(None);
    let url = url_from(None, &config).unwrap();
    assert_eq!(
        url,
        format!("sqlite:{}", config.workspace_dir.join("storage").display())
    );
    assert_eq!(explicit_url(&config), None, "nothing was configured");
    assert_eq!(
        explicit_url(&config_with(Some("memory"))).as_deref(),
        Some("memory")
    );
}

#[tokio::test]
async fn opens_memory_and_refuses_garbage() {
    let backend = open("memory").await.unwrap();
    assert_eq!(backend.driver(), "memory");
    assert!(open("ftp://nope").await.is_err());
}

#[test]
fn install_installed_and_clear() {
    // A private slot: installing into the process's would reroute every other
    // test's storage calls while this one runs.
    let slot = Slot::default();
    assert!(slot.installed().is_none());
    let backend: Arc<dyn StorageBackend> = Arc::new(tinystoragedrivers::MemoryStorage::new());
    assert!(slot.install(Arc::clone(&backend)).is_none());
    assert!(slot
        .installed()
        .is_some_and(|got| Arc::ptr_eq(&got, &backend)));
    assert!(slot
        .install(Arc::new(tinystoragedrivers::MemoryStorage::new()))
        .is_some());
    assert!(slot.clear());
    assert!(!slot.clear());
}

#[test]
fn agent_scopes_sit_under_the_local_tenant() {
    assert_eq!(scope_for_agent("agent-7").as_str(), "local/agent:agent-7");
    assert!(scope_for_agent("has a space")
        .as_str()
        .starts_with("local/agent:~"));
}

#[test]
fn the_scope_follows_the_acting_agent_and_fails_closed_in_saas() {
    assert_eq!(
        scope_from(None, Some("agent-7"), false).unwrap().as_str(),
        "local/agent:agent-7"
    );
    assert!(
        scope_from(None, Some("agent-7"), true).is_err(),
        "SaaS hands out profile scopes only"
    );
    assert!(scope_from(None, None, false).unwrap().is_local());
    assert!(
        scope_from(None, None, true).is_err(),
        "SaaS never falls back to a shared scope"
    );
}

#[test]
fn a_profile_scopes_storage_over_its_agents() {
    for saas in [false, true] {
        assert_eq!(
            scope_from(Some("u-alice"), Some("u-alice"), saas)
                .unwrap()
                .as_str(),
            "u:u-alice"
        );
        assert_eq!(
            scope_from(Some("u-alice"), None, saas).unwrap().as_str(),
            "u:u-alice"
        );
    }
    assert_ne!(scope_for_profile("a"), scope_for_profile("b"));
    let odd = scope_for_profile("has a space");
    assert!(odd.as_str().starts_with("u:~"), "{odd}");
    assert_ne!(odd, scope_for_profile("has  a space"));
    assert!(scope_for_profile(&"x".repeat(400))
        .as_str()
        .starts_with("u:~"));
}

#[test]
fn block_on_runs_a_future_from_sync_code() {
    let answer = block_on(async { Ok::<_, StorageError>(42) }).unwrap();
    assert_eq!(answer, 42);
    let error = block_on(async { Err::<(), _>(StorageError::conflict("raced")) }).unwrap_err();
    assert_eq!(error.message(), "raced");
}

#[tokio::test(flavor = "multi_thread")]
async fn block_on_works_inside_a_runtime_too() {
    let answer = tokio::task::spawn_blocking(|| block_on(async { Ok::<_, StorageError>(7) }))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(answer, 7);
}

#[test]
fn only_mongodb_is_shared_between_processes() {
    assert!(driver_is_shared("mongodb"));
    for driver in ["sqlite", "memory", "file"] {
        assert!(!driver_is_shared(driver), "{driver}");
    }
}

#[test]
fn only_mongodb_and_sqlite_have_cross_process_cas() {
    for driver in ["mongodb", "sqlite"] {
        assert!(driver_has_cross_process_cas(driver), "{driver}");
    }
    for driver in ["memory", "file", "", "postgres"] {
        assert!(!driver_has_cross_process_cas(driver), "{driver}");
    }
}
