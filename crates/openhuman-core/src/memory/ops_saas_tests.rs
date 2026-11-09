use super::*;
use crate::memory::test_fixtures::{bind_reference, config_in};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tinymemory_api::conformance::ReferenceEngine;
use tinymemory_api::{
    DocumentBody, EngineDescriptor, FetchRequest, ForgetReport, ForgetTarget, ItemKind,
    ListRequest, MemoryEngine, MemoryMeta, MetaFilter, Reach, RecallRequest, Role, SourceKind,
    SourceRef, StoreItem, StoreReceipt, Turn,
};

fn learn_params(text: &str) -> LearnParams {
    LearnParams {
        text: text.to_string(),
        kind: None,
        confidence: None,
        meta: None,
    }
}

#[test]
fn invalid_saas_root_is_refused_before_engine_operations() {
    const CHILD: &str = "OPENHUMAN_TEST_SAAS_MEMORY_STORE_GUARD";
    const TEST_NAME: &str =
        "memory::ops::saas_tests::invalid_saas_root_is_refused_before_engine_operations";

    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .contains("SAAS_MEMORY_STORE_GUARD_CHILD_PASSED"),
            "isolated SaaS test process failed or ran zero tests:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    crate::core::runtime::mode::lock_mode(crate::core::runtime::Mode::Saas).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for root in [None, Some("not a namespace!"), Some("root")] {
        let tmp = tempfile::tempdir().unwrap();
        let mut config = config_in(&tmp);
        config.memory.root = root.map(str::to_string);
        let calls = Arc::new(AtomicUsize::new(0));
        crate::memory::engine::install_test_engine(
            &config.workspace_dir,
            Arc::new(CountingEngine {
                inner: ReferenceEngine::new(),
                calls: calls.clone(),
            }),
        );
        let expected = "memory is unavailable: this agent has no valid memory root";
        let check = |error: Error| {
            assert!(matches!(error, Error::InvalidRequest(message) if message == expected));
        };
        let item = StoreItem::learning(
            "should not be stored",
            tinymemory_api::LearningKind::Fact,
            0.9,
            MemoryMeta::default(),
        );
        check(runtime.block_on(store_item(&config, item)).unwrap_err());
        check(
            runtime
                .block_on(recall(
                    &config,
                    RecallParams {
                        refers_to: None,
                        question: "anything".into(),
                        filter: None,
                        limit: None,
                    },
                ))
                .unwrap_err(),
        );
        check(
            runtime
                .block_on(fetch(
                    &config,
                    FetchParams {
                        refers_to: None,
                        query: "anything".into(),
                        mode: None,
                        filter: None,
                        limit: None,
                        cursor: None,
                    },
                ))
                .unwrap_err(),
        );
        check(
            runtime
                .block_on(items_list(&config, ItemsListParams::default()))
                .unwrap_err(),
        );
        check(
            runtime
                .block_on(forget(
                    &config,
                    ForgetParams {
                        ids: vec!["not-present".into()],
                        reach: None,
                    },
                ))
                .unwrap_err(),
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0, "root: {root:?}");
    }
    println!("SAAS_MEMORY_STORE_GUARD_CHILD_PASSED");
}

#[test]
fn valid_saas_identity_clamps_memory_operations_in_an_isolated_process() {
    const CHILD: &str = "OPENHUMAN_TEST_SAAS_MEMORY_CLAMP";
    const TEST_NAME: &str = "memory::ops::saas_tests::valid_saas_identity_clamps_memory_operations_in_an_isolated_process";

    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .contains("SAAS_MEMORY_CLAMP_CHILD_PASSED"),
            "isolated SaaS clamp test process failed or ran zero tests:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    crate::core::runtime::mode::lock_mode(crate::core::runtime::Mode::Saas).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let tmp = tempfile::tempdir().unwrap();
            let mut config = config_in(&tmp);
            config.memory.root = Some("team:acme".into());
            let engine = bind_reference(&config);
            let own = engine
                .store(StoreItem::learning(
                    "acme shared note",
                    tinymemory_api::LearningKind::Fact,
                    0.9,
                    MemoryMeta {
                        namespace: "team:acme".parse().unwrap(),
                        ..MemoryMeta::default()
                    },
                ))
                .await
                .unwrap()
                .id
                .0;
            let foreign = engine
                .store(StoreItem::learning(
                    "other team's private marker",
                    tinymemory_api::LearningKind::Fact,
                    0.9,
                    MemoryMeta {
                        namespace: "team:other".parse().unwrap(),
                        ..MemoryMeta::default()
                    },
                ))
                .await
                .unwrap()
                .id
                .0;

            for (namespace, text, repo) in [
                ("team:acme", "explore acme item", "org/alpha"),
                ("team:acme", "explore acme sibling", "org/beta"),
                ("team:other", "explore foreign item", "org/foreign"),
            ] {
                engine
                    .store(StoreItem::learning(
                        text,
                        tinymemory_api::LearningKind::Fact,
                        0.9,
                        MemoryMeta {
                            namespace: namespace.parse().unwrap(),
                            repo: Some(repo.into()),
                            tags: vec!["selected".into()],
                            ..MemoryMeta::default()
                        },
                    ))
                    .await
                    .unwrap();
            }
            engine
                .store(StoreItem::document(
                    "selected document",
                    MemoryMeta {
                        namespace: "team:acme".parse().unwrap(),
                        repo: Some("org/document".into()),
                        tags: vec!["selected".into()],
                        ..MemoryMeta::default()
                    },
                ))
                .await
                .unwrap();

            let explore_params = |scan_limit| crate::memory::explore::ExploreParams {
                facet: tinymemory_api::Facet::Repo,
                path: vec![crate::memory::explore::PathStep {
                    facet: tinymemory_api::Facet::Kind,
                    value: "learning".into(),
                }],
                filter: Some(MetaFilter {
                    tags_any: vec!["selected".into()],
                    ..MetaFilter::default()
                }),
                limit: Some(5),
                scan_limit,
            };
            let explored = crate::memory::explore::explore(&config, explore_params(Some(100)))
                .await
                .unwrap();
            assert_eq!(explored.total, 2);
            assert!(!explored.truncated);
            assert_eq!(
                explored
                    .buckets
                    .iter()
                    .map(|bucket| bucket.value.as_str())
                    .collect::<Vec<_>>(),
                ["org/alpha", "org/beta"]
            );
            let limited = crate::memory::explore::explore(&config, explore_params(Some(1)))
                .await
                .unwrap();
            assert_eq!(limited.total, 1);
            assert!(limited.truncated);

            let listed = items_list(&config, Default::default()).await.unwrap();
            assert_eq!(listed.items.len(), 4);
            assert!(listed.items.iter().any(|item| item.id.0 == own));
            assert!(!listed.items.iter().any(|item| item.id.0 == foreign));

            let fetched = fetch(
                &config,
                FetchParams {
                    refers_to: None,
                    query: "private marker".into(),
                    mode: None,
                    filter: None,
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap();
            assert!(!fetched.hits.iter().any(|hit| hit.id.0 == foreign));

            let recalled = recall(
                &config,
                RecallParams {
                    refers_to: None,
                    question: "private marker".into(),
                    filter: None,
                    limit: None,
                },
            )
            .await
            .unwrap();
            assert!(recalled
                .citations
                .iter()
                .all(|citation| citation.id.0 != foreign));

            let learned = learn(
                &config,
                learn_params("clamped learning"),
                Some(MemoryMeta {
                    namespace: "team:other".parse().unwrap(),
                    ..MemoryMeta::default()
                }),
            )
            .await
            .unwrap();
            let stored = store_item(
                &config,
                StoreItem::learning(
                    "clamped store",
                    tinymemory_api::LearningKind::Fact,
                    0.9,
                    MemoryMeta {
                        namespace: "team:other".parse().unwrap(),
                        ..MemoryMeta::default()
                    },
                ),
            )
            .await
            .unwrap();
            let own_items = engine
                .list(tinymemory_api::ListRequest {
                    filter: MetaFilter {
                        reach: Some(Reach::subtree("team:acme".parse().unwrap())),
                        ..MetaFilter::default()
                    },
                    limit: 20,
                    cursor: None,
                })
                .await
                .unwrap()
                .items;
            for id in [learned.id, stored.id.0] {
                let item = own_items.iter().find(|item| item.id.0 == id).unwrap();
                assert_eq!(item.meta.namespace, "team:acme".parse().unwrap());
            }

            let forgotten = forget(
                &config,
                ForgetParams {
                    ids: vec![foreign.clone()],
                    reach: None,
                },
            )
            .await
            .unwrap();
            assert_eq!(forgotten.forgotten, 0);
            let foreign_items = engine
                .list(tinymemory_api::ListRequest {
                    filter: MetaFilter {
                        reach: Some(Reach::subtree("team:other".parse().unwrap())),
                        ..MetaFilter::default()
                    },
                    limit: 20,
                    cursor: None,
                })
                .await
                .unwrap()
                .items;
            assert!(foreign_items.iter().any(|item| item.id.0 == foreign));

            let service_namespace: tinymemory_api::Namespace =
                "team:acme/service:newsletter".parse().unwrap();
            let source_id = "src-profile-cleanup";
            for namespace in ["team:acme", "team:other", "team:acme/service:newsletter"] {
                engine
                    .store(StoreItem::Document {
                        title: Some("source cleanup fixture".into()),
                        body: DocumentBody::Text("source cleanup fixture".into()),
                        mime: None,
                        meta: MemoryMeta {
                            namespace: namespace.parse().unwrap(),
                            source: SourceRef {
                                kind: SourceKind::Folder,
                                id: Some(source_id.into()),
                            },
                            ..MemoryMeta::default()
                        },
                    })
                    .await
                    .unwrap();
            }
            assert_eq!(
                crate::memory::sources::forget_items(&config, source_id)
                    .await
                    .unwrap(),
                2
            );
            let service_source_items = engine
                .list(ListRequest {
                    filter: MetaFilter {
                        reach: Some(Reach::exact(service_namespace.clone())),
                        source_id: Some(source_id.into()),
                        ..MetaFilter::default()
                    },
                    limit: 20,
                    cursor: None,
                })
                .await
                .unwrap()
                .items;
            // No explicit reach uses the engine's ordinary root subtree,
            // which leaves the service sandbox intact.
            assert_eq!(service_source_items.len(), 1);

            let thread_id = "thread-profile-cleanup";
            for namespace in ["team:acme", "team:other", "team:acme/service:newsletter"] {
                engine
                    .store(StoreItem::Conversation {
                        turns: vec![Turn::new(Role::User, "thread cleanup fixture")],
                        meta: MemoryMeta {
                            namespace: namespace.parse().unwrap(),
                            thread_id: Some(thread_id.into()),
                            ..MemoryMeta::default()
                        },
                    })
                    .await
                    .unwrap();
            }
            assert_eq!(
                crate::memory::deletion::forget_thread(&config, thread_id).await,
                2
            );
            let service_thread_items = engine
                .list(ListRequest {
                    filter: MetaFilter {
                        reach: Some(Reach::exact(service_namespace)),
                        thread_id: Some(thread_id.into()),
                        kinds: vec![ItemKind::Conversation],
                        ..MetaFilter::default()
                    },
                    limit: 20,
                    cursor: None,
                })
                .await
                .unwrap()
                .items;
            // Thread cleanup retains the same root-subtree boundary.
            assert_eq!(service_thread_items.len(), 1);
        });
    println!("SAAS_MEMORY_CLAMP_CHILD_PASSED");
}

struct CountingEngine {
    inner: ReferenceEngine,
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl MemoryEngine for CountingEngine {
    fn descriptor(&self) -> &EngineDescriptor {
        self.inner.descriptor()
    }

    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.health().await
    }

    async fn recall(
        &self,
        request: RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.recall(request).await
    }

    async fn fetch(
        &self,
        request: FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.fetch(request).await
    }

    async fn store(&self, item: StoreItem) -> tinymemory_api::Result<StoreReceipt> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.store(item).await
    }

    async fn forget(&self, target: ForgetTarget) -> tinymemory_api::Result<ForgetReport> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.forget(target).await
    }

    async fn list(&self, request: ListRequest) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.list(request).await
    }
}
