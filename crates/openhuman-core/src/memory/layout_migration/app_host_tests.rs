use std::sync::Arc;

use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::MemoryEngine;

use super::*;
use crate::memory::engine::install_test_engine_for_root;
use crate::memory::layout_migration::ops::scan;
use crate::memory::layout_migration::test_host::{count, fact};
use tinymemory_tools::layout_migration::{
    FsLegacyClaim, LayoutHost, LegacyClaim, Outcome, Trigger,
};

const ACCOUNT: &str = "0123456789abcdef01234567";

#[test]
fn memory_bind_errors_keep_their_engine_categories_at_the_migration_boundary() {
    assert_eq!(
        memory_error_to_engine(crate::memory::MemoryError::Off("memory is disabled".into())),
        tinymemory_api::Error::Off("memory is disabled".into())
    );
    assert_eq!(
        memory_error_to_engine(crate::memory::MemoryError::Unavailable("offline".into())),
        tinymemory_api::Error::Unavailable("offline".into())
    );
    assert_eq!(
        memory_error_to_engine(crate::memory::MemoryError::ConfigConflict(
            crate::core::StructuredRpcError {
                message: "configuration changed".into(),
                data: None,
                expected_user_state: true,
            }
        )),
        tinymemory_api::Error::Conflict("configuration changed".into())
    );
}

/// A config that lives where a signed-in account's does.
fn signed_in(tmp: &tempfile::TempDir) -> Config {
    let mut config = crate::memory::test_fixtures::config_in(tmp);
    config.config_path = tmp.path().join("users").join(ACCOUNT).join("config.toml");
    config
}

#[tokio::test]
async fn signed_out_there_is_nothing_to_move() {
    let tmp = tempfile::tempdir().unwrap();
    let config = crate::memory::test_fixtures::config_in(&tmp);
    assert!(matches!(
        AppHost::new(config.clone()).engines(),
        Err(lm::Error::Off(_))
    ));
    assert!(!scan(&AppHost::new(config.clone())).await.unwrap().needed);
}

#[tokio::test]
async fn binds_the_legacy_tree_and_the_users_own() {
    let tmp = tempfile::tempdir().unwrap();
    let config = signed_in(&tmp);
    let root = format!("org:{ACCOUNT}");
    let legacy = Arc::new(ReferenceEngine::new());
    let tree = Arc::new(ReferenceEngine::new());
    install_test_engine_for_root(&config.workspace_dir, None, legacy.clone());
    install_test_engine_for_root(&config.workspace_dir, Some(&root), tree.clone());
    legacy.store(fact("legacy fact")).await.unwrap();

    let engines = AppHost::new(config.clone()).engines().unwrap();
    assert_eq!(count(&*engines.legacy).await, 1);
    assert_eq!(count(&*engines.tree).await, 0);
    assert!(scan(&AppHost::new(config.clone())).await.unwrap().needed);

    let placement = AppHost::new(config.clone()).placement().unwrap();
    assert_eq!(placement.layout.root().to_string(), "root");
    assert_eq!(placement.chat_node.to_string(), "ws:main");
    assert!(matches!(placement.flows, FlowPlacement::WithRoot));
}

#[test]
fn places_into_the_v3_layout_before_the_switch() {
    let tmp = tempfile::tempdir().unwrap();
    let config = signed_in(&tmp);
    assert!(config.memory.layout != crate::config::MemoryLayoutMode::V3);
    let mut v3 = config.clone();
    v3.memory.layout = crate::config::MemoryLayoutMode::V3;
    let legacy = tinymemory_tools::layout::identity::Identity::root()
        .resolve(&crate::memory::scope::identity_pins(&config))
        .layout;
    let placement = AppHost::new(config.clone()).placement().unwrap();
    assert_ne!(placement.layout, legacy, "chats are pooled at ws:main");
    assert_eq!(
        placement.layout.conversations("a").unwrap(),
        placement.chat_node
    );
}

#[test]
fn every_account_on_a_self_hosted_engine_shares_one_claim() {
    let tmp = tempfile::tempdir().unwrap();
    let mut a = signed_in(&tmp);
    a.memory.engine = CORTEXDB_ENGINE.to_string();
    let mut b = a.clone();
    b.config_path = tmp
        .path()
        .join("users")
        .join("fedcba9876543210fedcba98")
        .join("config.toml");
    for config in [&a, &b] {
        install_test_engine_for_root(
            &config.workspace_dir,
            None,
            Arc::new(ReferenceEngine::new()),
        );
    }
    let key_a = claim(&a).unwrap().unwrap();
    let key_b = claim(&b).unwrap().unwrap();
    assert_eq!(key_a.marker_path(), key_b.marker_path());
    assert_eq!(
        key_a
            .marker_path()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap(),
        tmp.path()
    );
    assert_eq!(key_a.owner(), format!("user:{ACCOUNT}"));

    let hosted = signed_in(&tmp);
    assert!(claim(&hosted).unwrap().is_none());
}

#[tokio::test]
async fn the_switch_writes_the_migrated_persons_own_config_fresh() {
    let tmp = tempfile::tempdir().unwrap();
    let snapshot = signed_in(&tmp);
    std::fs::create_dir_all(snapshot.config_path.parent().unwrap()).unwrap();
    // A setting changed on disk after the run took its copy.
    let mut later = snapshot.clone();
    later.memory.agent_id = Some("changed-later".to_string());
    later.save().await.unwrap();

    AppHost::new(snapshot.clone()).switch().await.unwrap();

    let saved = Config::load_from_config_path(&snapshot.config_path, &snapshot.workspace_dir)
        .await
        .unwrap();
    assert_eq!(saved.memory.layout, crate::config::MemoryLayoutMode::V3);
    assert_eq!(saved.memory.agent_id.as_deref(), Some("changed-later"));
}

/// The host hands the module its claim: a self-hosted tree is not moved until
/// the user consents, and the account that did not claim it has nothing to
/// move. Fails if the adapter stops passing the claim on.
#[tokio::test]
async fn a_shared_tree_reaches_the_module_as_a_claim() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = signed_in(&tmp);
    config.memory.engine = CORTEXDB_ENGINE.to_string();
    let root = format!("org:{ACCOUNT}");
    let legacy = Arc::new(ReferenceEngine::new());
    install_test_engine_for_root(&config.workspace_dir, None, legacy.clone());
    install_test_engine_for_root(
        &config.workspace_dir,
        Some(&root),
        Arc::new(ReferenceEngine::new()),
    );
    legacy.store(fact("shared fact")).await.unwrap();

    let host = AppHost::new(config.clone());
    let view = scan(&host).await.unwrap();
    assert!(view.needed && view.shared, "{view:?}");
    let outcome = lm::run(&host, Trigger::Manual { takeover: false }, || async {
        false
    })
    .await
    .unwrap();
    assert_eq!(outcome, Outcome::NeedsTakeover);
    assert_eq!(count(&*host.engines().unwrap().tree).await, 0);

    // Another account on the machine took the tree first.
    let key = claim(&config).unwrap().unwrap();
    let app_dir = key
        .marker_path()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let other = FsLegacyClaim::new(app_dir, key.endpoint(), "user:fedcba9876543210fedcba98");
    assert!(other.take().unwrap());
    let view = scan(&host).await.unwrap();
    assert!(!view.needed && view.shared, "{view:?}");
}

#[test]
fn errors_keep_their_wire_codes_across_the_module_boundary() {
    for error in [
        Error::Off("x".into()),
        Error::Unsupported("x".into()),
        Error::InvalidRequest("x".into()),
        Error::NotFound("x".into()),
        Error::Config("x".into()),
        Error::Conflict("x".into()),
        Error::Unauthorized("x".into()),
        Error::Unavailable("x".into()),
        Error::InsufficientCredits("hosted refusal without a code marker".into()),
        Error::Engine("x".into()),
    ] {
        let back = from_module(to_module(error.clone()));
        assert_eq!(back.code(), error.code(), "{error:?}");
        assert_eq!(back.is_account_wide(), error.is_account_wide(), "{error:?}");
    }

    let credit_error = Error::InsufficientCredits("hosted refusal without a code marker".into());
    let back = from_module(to_module(credit_error));
    assert!(matches!(back, Error::InsufficientCredits(_)));
    assert!(back.is_account_wide());

    let plan_error = crate::memory::MemoryError::PlanRequired(
        "[MEMORY_PLAN_REQUIRED] memory API memory/scopes (HTTP 403)".into(),
    );
    let module_error = to_module(memory_error_to_engine(plan_error));
    let back = crate::memory::MemoryError::from(from_module(module_error));
    assert_eq!(back.code(), crate::memory::error::PLAN_REQUIRED);
    assert!(back.is_account_wide());
}

#[test]
fn one_endpoint_has_one_marker_however_it_is_spelled() {
    let dir = tempfile::tempdir().unwrap();
    let local = "http://127.0.0.1:3141";
    let key = FsLegacyClaim::new(dir.path(), local, "user:a");
    for spelling in ["HTTP://127.0.0.1:3141/", " http://127.0.0.1:3141 "] {
        assert_eq!(
            FsLegacyClaim::new(dir.path(), spelling, "user:b").marker_path(),
            key.marker_path()
        );
    }
    assert_ne!(
        FsLegacyClaim::new(dir.path(), "http://127.0.0.1:3142", "user:a").marker_path(),
        key.marker_path()
    );
}

#[test]
fn the_first_account_takes_the_tree_and_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let a = FsLegacyClaim::new(dir.path(), "http://127.0.0.1:3141", "user:a");
    let b = FsLegacyClaim::new(dir.path(), "http://127.0.0.1:3141", "user:b");
    assert!(a.take().unwrap());
    assert!(!b.take().unwrap());
    assert!(b.held_by_other().unwrap());
    assert!(a.take().unwrap());
    assert!(!a.held_by_other().unwrap());
}

#[test]
fn an_unreadable_marker_is_an_error_not_a_free_tree() {
    let dir = tempfile::tempdir().unwrap();
    let claim = FsLegacyClaim::new(dir.path(), "http://127.0.0.1:3141", "user:a");
    std::fs::create_dir_all(claim.marker_path().parent().unwrap()).unwrap();
    std::fs::write(claim.marker_path(), b"{ torn").unwrap();
    assert!(claim.held_by_other().is_err());
    assert!(claim.take().is_err());
}
