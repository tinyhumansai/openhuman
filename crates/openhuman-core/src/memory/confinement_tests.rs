use std::sync::Arc;

use tinymemory_api::{
    Error, LearningKind, MemoryEngine, MemoryMeta, MetaFilter, Namespace, Reach, SourceKind,
    SourceRef, StoreItem,
};
use tinymemory_tools::confine::{Confinement, Policy};
use tinymemory_tools::layout::identity::Identity;

use crate::memory::explore::{self, ItemsGetParams, PathStep};
use crate::memory::test_fixtures::{bind_reference, config_in};
use crate::memory::types::{FetchParams, ForgetParams, ItemsListParams, LearnParams, RecallParams};

fn ns(raw: &str) -> Namespace {
    raw.parse().unwrap()
}

fn confinement(config: &crate::config::Config, identity: &Identity) -> Confinement {
    let resolved = identity.resolve(&crate::memory::scope::identity_pins(config));
    Confinement {
        allowed: Reach::subtree(resolved.root().clone()),
        policy: Policy::Refuse,
    }
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

async fn two_roots(tmp: &tempfile::TempDir) -> (crate::config::Config, String, String) {
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
fn rpc_refuse_uses_the_identity_subtree_and_rejects_wider_reaches() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.root = Some("project:q4".to_string());
    let rpc = confinement(&config, &Identity::root());
    assert_eq!(rpc.allowed, Reach::subtree(ns("project:q4")));
    assert_eq!(rpc.reach(None).unwrap(), Reach::subtree(ns("project:q4")));

    let narrow = Reach::exact(ns("project:q4/agent:writer"));
    assert_eq!(rpc.reach(Some(narrow.clone())).unwrap(), narrow);
    for wider in [
        Reach::subtree(Namespace::ROOT),
        Reach::subtree(ns("team:other")),
        Reach::of(ns("project:q4/agent:writer")),
    ] {
        assert!(matches!(
            rpc.reach(Some(wider)),
            Err(Error::InvalidRequest(_))
        ));
    }

    let team = confinement(&config_in(&tmp), &Identity::team_member("acme", "scout"));
    assert_eq!(team.allowed, Reach::subtree(ns("team:acme")));
}

#[tokio::test]
async fn rpc_list_fetch_and_recall_never_read_another_root() {
    let tmp = tempfile::tempdir().unwrap();
    let (config, mine, theirs) = two_roots(&tmp).await;
    let rpc = confinement(&config, &Identity::root());

    let listed = crate::memory::ops::items_list(
        &config,
        ItemsListParams {
            filter: Some(rpc.filter(None).unwrap()),
            ..ItemsListParams::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(ids(&listed.items), std::slice::from_ref(&mine));

    let page = crate::memory::ops::fetch(
        &config,
        FetchParams {
            refers_to: None,
            query: "plan".into(),
            mode: None,
            filter: Some(rpc.filter(None).unwrap()),
            limit: Some(100),
            cursor: None,
        },
    )
    .await
    .unwrap();
    assert!(!ids(&page.hits).contains(&theirs), "{:?}", ids(&page.hits));

    let answer = crate::memory::ops::recall(
        &config,
        RecallParams {
            refers_to: None,
            question: "secret plan".into(),
            filter: Some(rpc.filter(None).unwrap()),
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
async fn rpc_filter_and_explorer_paths_outside_the_identity_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (config, _, _) = two_roots(&tmp).await;
    let rpc = confinement(&config, &Identity::root());
    let outside = MetaFilter {
        reach: Some(Reach::subtree(ns("team:other"))),
        ..MetaFilter::default()
    };
    assert!(matches!(
        rpc.filter(Some(outside.clone())),
        Err(Error::InvalidRequest(_))
    ));

    let path = vec![PathStep {
        facet: tinymemory_api::Facet::Namespace,
        value: "team:other".to_string(),
    }];
    let narrowed = explore::narrowed(None, &path).unwrap();
    assert!(matches!(
        rpc.filter(Some(narrowed)),
        Err(Error::InvalidRequest(_))
    ));

    let response = crate::memory::ops::items_list(
        &config,
        ItemsListParams {
            filter: Some(rpc.filter(None).unwrap()),
            ..ItemsListParams::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(response.items.len(), 1);
}

#[tokio::test]
async fn rpc_items_get_and_forget_cannot_reach_another_root() {
    let tmp = tempfile::tempdir().unwrap();
    let (config, _, theirs) = two_roots(&tmp).await;
    let rpc = confinement(&config, &Identity::root());

    let get = explore::items_get(
        &config,
        ItemsGetParams {
            ids: vec![theirs.clone()],
            reach: Some(rpc.reach(None).unwrap()),
        },
    )
    .await
    .unwrap();
    assert!(get.items.is_empty());

    let forgotten = crate::memory::ops::forget(
        &config,
        ForgetParams {
            ids: vec![theirs.clone()],
            reach: Some(rpc.reach(None).unwrap()),
        },
    )
    .await
    .unwrap();
    assert_eq!(forgotten.forgotten, 0);

    let engine: Arc<dyn MemoryEngine> = crate::memory::engine::resolve(&config)
        .await
        .engine()
        .unwrap()
        .engine;
    assert_eq!(
        ids(&engine
            .list(tinymemory_api::ListRequest {
                filter: MetaFilter {
                    reach: Some(Reach::subtree(ns("team:other"))),
                    ..MetaFilter::default()
                },
                limit: 10,
                cursor: None,
            })
            .await
            .unwrap()
            .items)
        .as_slice(),
        [theirs]
    );
}

#[tokio::test]
async fn rpc_learn_uses_the_layout_default_and_refuses_an_explicit_outside_namespace() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.root = Some("team:acme".to_string());
    let engine = bind_reference(&config);
    let identity = Identity::root().resolve(&crate::memory::scope::identity_pins(&config));
    let rpc = confinement(&config, &Identity::root());
    let default_meta = rpc.learn_meta(None, identity.layout.learnings()).unwrap();
    let result = crate::memory::ops::learn(
        &config,
        LearnParams {
            text: "my learning".into(),
            kind: None,
            confidence: None,
            meta: Some(default_meta.clone()),
        },
        None,
    )
    .await
    .unwrap();
    let item = engine
        .list(tinymemory_api::ListRequest {
            filter: MetaFilter {
                reach: Some(Reach::subtree(ns("team:acme"))),
                ..MetaFilter::default()
            },
            limit: 10,
            cursor: None,
        })
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|item| item.id.0 == result.id)
        .unwrap();
    assert_eq!(item.meta.namespace, default_meta.namespace);

    let mut outside = MemoryMeta::default();
    outside.namespace = ns("team:other");
    assert!(matches!(
        rpc.learn_meta(Some(outside), identity.layout.learnings()),
        Err(Error::InvalidRequest(_))
    ));
}
