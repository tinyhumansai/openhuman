use super::*;

fn config_in(dir: &tempfile::TempDir) -> Config {
    Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    }
}

#[test]
fn wire_conversion_round_trips_every_flag() {
    for read in [false, true] {
        for write in [false, true] {
            for admin in [false, true] {
                let pref = UserScopePref { read, write, admin };
                assert_eq!(from_wire(to_wire(pref)), pref);
            }
        }
    }
}

#[test]
fn row_key_normalises_the_toolkit() {
    assert_eq!(UserScopePref::key(" GitHub "), "github");
    assert_eq!(
        UserScopePref::key("   "),
        "",
        "an all-whitespace toolkit has no row"
    );
}

#[tokio::test]
async fn save_rejects_an_empty_toolkit() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    assert!(save(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "  ",
        UserScopePref::default()
    )
    .await
    .is_err());
}

/// A stored preference is never lost: either the module took it (the read
/// returns it and the stored value is emptied) or the module was unreachable
/// (the read fails closed and the value is left exactly as it was so the
/// migration retries).
#[tokio::test]
async fn a_legacy_pref_either_reaches_the_module_or_stays_stored() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    let legacy = serde_json::json!({"github":{"read":true,"write":false,"admin":true}});
    file_store::save(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        USER_SCOPES_FILE,
        &legacy,
    )
    .await
    .unwrap();
    let stored = UserScopePref {
        read: true,
        write: false,
        admin: true,
    };

    let pref = load(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "github",
    )
    .await;
    let done = migrated().lock().unwrap().contains(&config.workspace_dir);
    let left: LegacyScopeFile = file_store::load(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        USER_SCOPES_FILE,
    )
    .await
    .unwrap();
    if done {
        assert_eq!(pref, Ok(stored), "the migrated row must decide visibility");
        assert!(left.is_empty(), "the stored value is emptied once moved");
    } else {
        assert!(
            pref.is_err(),
            "an unreachable module fails closed: {pref:?}"
        );
        assert_eq!(left.get("github"), Some(&stored));
    }
}

#[tokio::test]
async fn load_rejects_an_empty_toolkit_instead_of_answering_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    assert!(
        load(&crate::core::runtime::DispatchCtx::ambient(), &config, "  ")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_workspace_with_no_legacy_file_is_marked_migrated() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    migrate_legacy(&crate::core::runtime::DispatchCtx::ambient(), &config).await;
    assert!(migrated().lock().unwrap().contains(&config.workspace_dir));
}

/// A save made while the legacy migration is still deferred (another row keeps
/// failing) must not be undone by the next migration pass re-pushing the
/// toolkit's stale legacy row.
#[tokio::test]
async fn a_saved_pref_survives_a_deferred_legacy_migration() {
    use crate::integrations::composio::module_client::module_guard;
    let _serialised = module_guard().await;
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    file_store::save(
        &ctx,
        &config,
        USER_SCOPES_FILE,
        &serde_json::json!({
            "gmail": {"read": true, "write": true, "admin": true},
            "slack": {"read": true, "write": true, "admin": false},
        }),
    )
    .await
    .unwrap();
    FAIL_MIGRATING.with(|fail| *fail.borrow_mut() = Some("slack".into()));

    let revoked = UserScopePref {
        read: true,
        write: true,
        admin: false,
    };
    let saved = save(&ctx, &config, "gmail", revoked).await;
    let pref = load(&ctx, &config, "gmail").await;
    let left: LegacyScopeFile = file_store::load(&ctx, &config, USER_SCOPES_FILE)
        .await
        .unwrap();

    FAIL_MIGRATING.with(|fail| *fail.borrow_mut() = None);
    let _ = save(&ctx, &config, "gmail", UserScopePref::default()).await;
    let _ = save(&ctx, &config, "slack", UserScopePref::default()).await;

    saved.expect("the module stores the row");
    assert_eq!(pref, Ok(revoked), "a revoked admin scope came back");
    assert!(
        !left.contains_key("gmail"),
        "the saved toolkit's legacy row stays"
    );
    assert!(
        left.contains_key("slack"),
        "an unmigrated row is kept for retry"
    );
}

/// When the legacy row can't be dropped (an unreadable legacy file), the save
/// reports failure, so the module must keep the row it enforced before rather
/// than the one the UI is about to roll back.
#[tokio::test]
async fn a_save_whose_legacy_cleanup_fails_leaves_the_prior_row() {
    use crate::integrations::composio::module_client::module_guard;
    let _serialised = module_guard().await;
    let ctx = crate::core::runtime::DispatchCtx::ambient();
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    let granted = UserScopePref {
        read: true,
        write: true,
        admin: true,
    };
    connectors::call::<_, ComposioUserScopesResponse>(
        &config,
        methods::SET_USER_SCOPES,
        ComposioSetUserScopesRequest {
            toolkit: "gmail".into(),
            scopes: to_wire(granted),
        },
    )
    .await
    .unwrap();
    file_store::save(
        &ctx,
        &config,
        USER_SCOPES_FILE,
        &serde_json::json!({"gmail": "not a pref"}),
    )
    .await
    .unwrap();

    let saved = save(&ctx, &config, "gmail", UserScopePref::default()).await;
    let enforced = connectors::call::<_, ComposioUserScopesResponse>(
        &config,
        methods::GET_USER_SCOPES,
        ComposioGetUserScopesRequest {
            toolkit: "gmail".into(),
        },
    )
    .await
    .map(|reply| from_wire(reply.scopes));

    file_store::save(&ctx, &config, USER_SCOPES_FILE, &serde_json::json!({}))
        .await
        .unwrap();
    let _ = save(&ctx, &config, "gmail", UserScopePref::default()).await;

    assert!(saved.is_err(), "the save must report the failed cleanup");
    assert_eq!(enforced.unwrap(), granted, "the module keeps the prior row");
}
