use super::*;

use tinymemory_api::{LearningKind, MemoryEngine, SourceKind, SourceRef, StoreItem};

use crate::memory::error::INVALID_REQUEST;
use crate::memory::explore::PathStep;
use crate::memory::scope::MemoryIdentity;
use crate::memory::test_fixtures::{bind_reference, config_in};

fn ns(value: &str) -> Namespace {
    value.parse().unwrap()
}

fn learning_at(text: &str, namespace: &str) -> StoreItem {
    StoreItem::learning(
        text,
        LearningKind::Fact,
        0.9,
        MemoryMeta {
            namespace: ns(namespace),
            source: SourceRef {
                kind: SourceKind::Agent,
                id: None,
            },
            ..MemoryMeta::default()
        },
    )
}

/// A config whose host binding roots memory at `team:acme`, and an engine
/// holding one learning inside that root and one in a sibling root.
async fn two_roots(tmp: &tempfile::TempDir) -> (Config, String, String) {
    let mut config = config_in(tmp);
    config.memory.root = Some("team:acme".to_string());
    let engine = bind_reference(&config);
    let mine = engine
        .store(learning_at("acme ships on fridays", "team:acme"))
        .await
        .unwrap()
        .id
        .0;
    let theirs = engine
        .store(learning_at("other team secret plan", "team:other"))
        .await
        .unwrap()
        .id
        .0;
    (config, mine, theirs)
}

fn ids(hits: &[tinymemory_api::Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.id.0.clone()).collect()
}

#[test]
fn an_unset_reach_becomes_the_allowed_one_and_a_wider_one_is_refused() {
    let allowed = Reach::subtree(ns("team:acme"));
    assert_eq!(confine_reach(None, &allowed).unwrap(), allowed);
    let narrower = Reach::exact(ns("team:acme/agent:writer"));
    assert_eq!(
        confine_reach(Some(narrower.clone()), &allowed).unwrap(),
        narrower
    );
    for wider in [
        Reach::subtree(Namespace::ROOT),
        Reach::subtree(ns("team:other")),
        Reach::of(ns("team:acme/agent:writer")),
    ] {
        let error = confine_reach(Some(wider), &allowed).unwrap_err();
        assert_eq!(error.code(), INVALID_REQUEST, "{error}");
    }
}

#[test]
fn the_allowed_reach_is_the_identity_roots_subtree() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    assert_eq!(
        allowed_reach(&config, &MemoryIdentity::root()),
        Reach::subtree(Namespace::ROOT)
    );
    config.memory.root = Some("project:q4".to_string());
    assert_eq!(
        allowed_reach(&config, &MemoryIdentity::root()),
        Reach::subtree(ns("project:q4"))
    );
}

#[test]
fn a_team_members_reach_is_its_teams_subtree() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let reach = allowed_reach(&config, &MemoryIdentity::team_member("acme", "scout"));
    assert_eq!(reach, Reach::subtree(ns("team:acme")));
}

#[tokio::test]
async fn list_fetch_and_recall_never_read_another_root() {
    let tmp = tempfile::tempdir().unwrap();
    let (config, mine, theirs) = two_roots(&tmp).await;

    let listed = items_list(&config, &MemoryIdentity::root(), ItemsListParams::default())
        .await
        .unwrap();
    assert_eq!(ids(&listed.items), std::slice::from_ref(&mine));

    let page = fetch(
        &config,
        &MemoryIdentity::root(),
        FetchParams {
            refers_to: None,
            query: "plan".into(),
            mode: None,
            filter: None,
            limit: Some(100),
            cursor: None,
        },
    )
    .await
    .unwrap();
    assert!(!ids(&page.hits).contains(&theirs), "{:?}", ids(&page.hits));

    let answer = recall(
        &config,
        &MemoryIdentity::root(),
        RecallParams {
            refers_to: None,
            question: "secret plan".into(),
            filter: None,
            limit: Some(10),
        },
    )
    .await
    .unwrap();
    assert!(answer
        .citations
        .iter()
        .all(|citation| citation.id.0 != theirs));
}

#[tokio::test]
async fn a_caller_reach_into_another_root_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (config, _, _) = two_roots(&tmp).await;
    let elsewhere = MetaFilter {
        reach: Some(Reach::subtree(ns("team:other"))),
        ..MetaFilter::default()
    };

    let error = items_list(
        &config,
        &MemoryIdentity::root(),
        ItemsListParams {
            filter: Some(elsewhere.clone()),
            ..ItemsListParams::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), INVALID_REQUEST);

    let error = fetch(
        &config,
        &MemoryIdentity::root(),
        FetchParams {
            refers_to: None,
            query: "plan".into(),
            mode: None,
            filter: Some(elsewhere.clone()),
            limit: None,
            cursor: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), INVALID_REQUEST);

    let error = recall(
        &config,
        &MemoryIdentity::root(),
        RecallParams {
            refers_to: None,
            question: "plan".into(),
            filter: Some(elsewhere),
            limit: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), INVALID_REQUEST);
}

#[tokio::test]
async fn an_explorer_path_into_another_root_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (config, _, _) = two_roots(&tmp).await;
    let path = vec![PathStep {
        facet: tinymemory_api::Facet::Namespace,
        value: "team:other".to_string(),
    }];

    let error = items_list(
        &config,
        &MemoryIdentity::root(),
        ItemsListParams {
            path: path.clone(),
            ..ItemsListParams::default()
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), INVALID_REQUEST);

    let error = explore(
        &config,
        &MemoryIdentity::root(),
        ExploreParams {
            facet: tinymemory_api::Facet::Kind,
            path,
            filter: None,
            limit: None,
            scan_limit: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), INVALID_REQUEST);

    // Exploring inside the root still works and counts only its items.
    let page = explore(
        &config,
        &MemoryIdentity::root(),
        ExploreParams {
            facet: tinymemory_api::Facet::Kind,
            path: Vec::new(),
            filter: None,
            limit: None,
            scan_limit: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(page.total, 1);
}

#[tokio::test]
async fn items_get_and_forget_leave_another_roots_items_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let (config, mine, theirs) = two_roots(&tmp).await;

    let read = items_get(
        &config,
        &MemoryIdentity::root(),
        ItemsGetParams {
            ids: vec![mine.clone(), theirs.clone()],
            reach: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(ids(&read.items), std::slice::from_ref(&mine));

    let error = items_get(
        &config,
        &MemoryIdentity::root(),
        ItemsGetParams {
            ids: vec![theirs.clone()],
            reach: Some(Reach::subtree(Namespace::ROOT)),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), INVALID_REQUEST);

    let forgotten = forget(
        &config,
        &MemoryIdentity::root(),
        ForgetParams {
            ids: vec![theirs.clone()],
            reach: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(forgotten.forgotten, 0);

    let error = forget(
        &config,
        &MemoryIdentity::root(),
        ForgetParams {
            ids: vec![theirs.clone()],
            reach: Some(Reach::subtree(ns("team:other"))),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), INVALID_REQUEST);

    // Still there for its own root.
    let mut other = config.clone();
    other.memory.root = Some("team:other".to_string());
    let still = items_get(
        &other,
        &MemoryIdentity::root(),
        ItemsGetParams {
            ids: vec![theirs.clone()],
            reach: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(ids(&still.items), [theirs]);

    // Its own items are forgotten as before.
    let forgotten = forget(
        &config,
        &MemoryIdentity::root(),
        ForgetParams {
            ids: vec![mine],
            reach: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(forgotten.forgotten, 1);
}

#[tokio::test]
async fn learn_lands_inside_the_root_and_refuses_another() {
    let tmp = tempfile::tempdir().unwrap();
    let (config, _, _) = two_roots(&tmp).await;

    let learned = learn(
        &config,
        &MemoryIdentity::root(),
        LearnParams {
            text: "acme prefers short standups".into(),
            kind: None,
            confidence: None,
            meta: None,
        },
    )
    .await
    .unwrap();
    let read = items_get(
        &config,
        &MemoryIdentity::root(),
        ItemsGetParams {
            ids: vec![learned.id],
            reach: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(read.items.len(), 1);
    assert!(Reach::subtree(ns("team:acme")).admits(&read.items[0].meta.namespace));

    let error = learn(
        &config,
        &MemoryIdentity::root(),
        LearnParams {
            text: "planted in another team".into(),
            kind: None,
            confidence: None,
            meta: Some(MemoryMeta {
                namespace: ns("team:other"),
                ..MemoryMeta::default()
            }),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), INVALID_REQUEST);
}
