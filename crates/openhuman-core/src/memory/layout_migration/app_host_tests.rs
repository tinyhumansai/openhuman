use std::sync::Arc;

use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::MemoryEngine;

use super::*;
use crate::memory::engine::install_test_engine_for_root;
use crate::memory::layout_migration::claim;
use crate::memory::layout_migration::service::scan;
use crate::memory::layout_migration::test_host::{count, fact};
use tinymemory_tools::layout_migration::{LayoutHost, Outcome, Trigger};

const ACCOUNT: &str = "0123456789abcdef01234567";

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
    assert!(placement.chat_node.to_string().ends_with("ws:main"));
    assert!(matches!(placement.flows, FlowPlacement::WithRoot));
}

#[test]
fn places_into_the_v3_layout_before_the_switch() {
    let tmp = tempfile::tempdir().unwrap();
    let config = signed_in(&tmp);
    assert!(!crate::memory::scope::layout_is_v3(&config));
    let mut v3 = config.clone();
    v3.memory.layout = crate::config::MemoryLayoutMode::V3;
    let legacy = crate::memory::scope::MemoryIdentity::root()
        .resolve(&config)
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
    let key_a = claim_key(&a).unwrap().unwrap();
    let key_b = claim_key(&b).unwrap().unwrap();
    assert_eq!(key_a.marker, key_b.marker);
    assert_eq!(
        key_a
            .marker
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap(),
        tmp.path()
    );
    assert_eq!(key_a.owner, format!("user:{ACCOUNT}"));

    let hosted = signed_in(&tmp);
    assert!(claim_key(&hosted).unwrap().is_none());
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
    assert!(crate::memory::scope::layout_is_v3(&saved));
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
    let key = claim_key(&config).unwrap().unwrap();
    let other = ClaimKey::new(
        key.marker
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap(),
        &key.endpoint,
        "user:fedcba9876543210fedcba98",
    );
    assert!(claim::take(&other).unwrap());
    let view = scan(&host).await.unwrap();
    assert!(!view.needed && view.shared, "{view:?}");
}

#[test]
fn errors_keep_their_wire_codes_across_the_module_boundary() {
    for error in [
        MemoryError::Off("x".into()),
        MemoryError::Unsupported("x".into()),
        MemoryError::invalid("x"),
        MemoryError::Unauthorized("x".into()),
        MemoryError::Unavailable("x".into()),
        MemoryError::InsufficientCredits("[USER_INSUFFICIENT_CREDITS] x".into()),
        MemoryError::Engine("x".into()),
    ] {
        let back = from_module(to_module(error.clone()));
        assert_eq!(back.code(), error.code(), "{error:?}");
        assert_eq!(back.is_account_wide(), error.is_account_wide(), "{error:?}");
    }
}
