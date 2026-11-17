use super::*;
use std::collections::BTreeMap;

fn config_in(dir: &tempfile::TempDir) -> Config {
    crate::storage::local::use_persistent_default_for_test(dir.path());
    Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    }
}

#[tokio::test]
async fn missing_value_loads_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let loaded: BTreeMap<String, u32> = load(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config_in(&dir),
        IDENTITIES_FILE,
    )
    .await
    .unwrap();
    assert!(loaded.is_empty());
}

#[tokio::test]
async fn save_then_load_round_trips_in_the_workspace_default_backend() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    let mut value = BTreeMap::new();
    value.insert("gmail".to_string(), 3u32);
    save(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        USER_SCOPES_FILE,
        &value,
    )
    .await
    .unwrap();
    let loaded: BTreeMap<String, u32> = load(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        USER_SCOPES_FILE,
    )
    .await
    .unwrap();
    assert_eq!(loaded, value);
    // The other value is untouched, and nothing was written as a JSON file.
    let other: BTreeMap<String, u32> = load(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        IDENTITIES_FILE,
    )
    .await
    .unwrap();
    assert!(other.is_empty());
    assert!(!dir.path().join("integrations").exists());
    assert!(dir.path().join("storage").exists());
}

#[tokio::test]
async fn update_applies_and_persists_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    let bump = |v: &mut BTreeMap<String, u32>| {
        *v.entry("n".into()).or_default() += 1;
        ((), true)
    };
    update(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        IDENTITIES_FILE,
        bump,
    )
    .await
    .unwrap();
    update(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        IDENTITIES_FILE,
        bump,
    )
    .await
    .unwrap();
    let loaded: BTreeMap<String, u32> = load(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        IDENTITIES_FILE,
    )
    .await
    .unwrap();
    assert_eq!(loaded["n"], 2);
}

#[tokio::test]
async fn a_legacy_state_file_is_imported_once_and_left_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    let config = config_in(&dir);
    let legacy_dir = dir.path().join("integrations");
    std::fs::create_dir_all(&legacy_dir).unwrap();
    std::fs::write(legacy_dir.join(USER_SCOPES_FILE), r#"{"gmail":7}"#).unwrap();

    let loaded: BTreeMap<String, u32> = load(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        USER_SCOPES_FILE,
    )
    .await
    .unwrap();
    assert_eq!(loaded["gmail"], 7);
    // The legacy file is read only: same name, same bytes, no `.migrated`.
    assert_eq!(
        std::fs::read(legacy_dir.join(USER_SCOPES_FILE)).unwrap(),
        br#"{"gmail":7}"#
    );
    assert!(!legacy_dir
        .join(format!("{USER_SCOPES_FILE}.migrated"))
        .exists());

    // A later edit is not clobbered by a re-import.
    update(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        USER_SCOPES_FILE,
        |v: &mut BTreeMap<String, u32>| {
            v.insert("gmail".into(), 9);
            ((), true)
        },
    )
    .await
    .unwrap();
    let again: BTreeMap<String, u32> = load(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        USER_SCOPES_FILE,
    )
    .await
    .unwrap();
    assert_eq!(again["gmail"], 9);
    assert_eq!(
        std::fs::read(legacy_dir.join(USER_SCOPES_FILE)).unwrap(),
        br#"{"gmail":7}"#
    );
}

fn agent_ctx(agent: &str) -> crate::core::runtime::DispatchCtx {
    use crate::core::runtime::{ContextOverlay, CoreContext, DispatchCtx, DomainSet};
    DispatchCtx::new(
        CoreContext::for_test(DomainSet::full(), None).derive_with(
            ContextOverlay::new(Config::default(), DomainSet::full(), Default::default())
                .session_agent(agent),
        ),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stores_driven_by_two_handed_contexts_do_not_cross() {
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (config_a, config_b) = (config_in(&dir_a), config_in(&dir_b));
    let (asha, ravi) = (agent_ctx("u-asha"), agent_ctx("u-ravi"));
    let bump = |by: u32| {
        move |map: &mut BTreeMap<String, u32>| {
            *map.entry("n".into()).or_default() += by;
            ((), true)
        }
    };
    // Called from outside any scope: each call acts for the context it was
    // handed, over its own workspace.
    let (a, b) = tokio::join!(
        update(&asha, &config_a, IDENTITIES_FILE, bump(1)),
        update(&ravi, &config_b, IDENTITIES_FILE, bump(10)),
    );
    a.unwrap();
    b.unwrap();
    let from_a: BTreeMap<String, u32> = load(&asha, &config_a, IDENTITIES_FILE).await.unwrap();
    let from_b: BTreeMap<String, u32> = load(&ravi, &config_b, IDENTITIES_FILE).await.unwrap();
    assert_eq!(from_a.get("n"), Some(&1));
    assert_eq!(from_b.get("n"), Some(&10));
}
