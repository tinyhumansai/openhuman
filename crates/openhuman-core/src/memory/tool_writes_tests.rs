use super::*;
use crate::memory::test_fixtures::{bind_reference, config_in, stored, RefusingEngine};
use crate::memory::tools::CallFacts;
use serde_json::json;
use tinymemory_api::{MetaFilter, Namespace, Reach};
use tinymemory_tools::{CallContext, SingleTool, ToolScope};

/// Exercises the production native single-tool parser and the host write sink.
pub(crate) fn enqueue(
    config: &Config,
    args: &serde_json::Value,
    facts: &CallFacts,
) -> tinymemory_api::Result<serde_json::Value> {
    let config = config.clone();
    let args = args.clone();
    let facts = facts.clone();
    let invoke = move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(async move {
                let scope = ToolScope {
                    place: facts.namespace.clone(),
                    reach: Some(facts.reach.clone()),
                    writes: true,
                    zone: Some(config.time_zone()),
                    workspace: facts.workspace.clone(),
                    thread_id: facts.thread_id.clone(),
                    agent_id: facts.agent_id.clone(),
                    tool_call: Some(tinymemory_api::ToolCallRef {
                        name: crate::memory::tools::MEMORY_TOOL_NAME.to_string(),
                        id: facts.tool_call_id.clone(),
                    }),
                };
                let single = SingleTool::queued(
                    crate::memory::tools::MEMORY_TOOL_NAME,
                    scope,
                    std::sync::Arc::new(super::HostWriteSink::new(std::sync::Arc::new(config))),
                );
                let outcome = single
                    .call_with_details(args, CallContext::default())
                    .await?;
                Ok(outcome.value)
            })
    };
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::spawn(invoke)
            .join()
            .map_err(|_| tinymemory_api::Error::Unavailable("native test call panicked".into()))?
    } else {
        invoke()
    }
}

pub(crate) fn queued_for(config: &Config, facts: &CallFacts) -> tinymemory_api::Result<Vec<Entry>> {
    super::read_at(&super::queue_path(config, &facts.reach.at))
}

pub(crate) async fn drain_for(config: &Config, root: &Namespace) -> usize {
    let file = super::queue_path(config, root);
    let owner = super::owner_state(config);
    let _owner = owner.barrier.read().await;
    let state = super::drain_state(&file);
    let _draining = state.barrier.lock().await;
    let generation = owner.gate.generation();
    super::drain_locked(config, &file, &owner, generation).await
}

fn facts(config: &Config) -> CallFacts {
    CallFacts::of(
        config,
        &tinymemory_tools::layout::identity::Identity::agent("outbox-agent"),
    )
}

#[test]
fn owner_states_are_released_after_the_last_write_holder_drops() {
    let temp = tempfile::tempdir().unwrap();
    let mut keys = Vec::new();
    let mut queue_files = Vec::new();
    let states = (0..32)
        .map(|index| {
            let workspace = temp.path().join(index.to_string());
            std::fs::create_dir_all(&workspace).unwrap();
            let config = config_in(&temp);
            let config = Config {
                workspace_dir: workspace.clone(),
                ..config
            };
            let file = workspace
                .join("memory")
                .join(format!("{}-writes.json", owner_prefix(&config)));
            keys.push(workspace.join("memory").join(owner_prefix(&config)));
            queue_files.push(file.clone());
            drop(queue(&file));
            owner_state(&config)
        })
        .collect::<Vec<_>>();

    drop(states);
    let sentinel_workspace = temp.path().join("sentinel");
    std::fs::create_dir_all(&sentinel_workspace).unwrap();
    let sentinel_config = Config {
        workspace_dir: sentinel_workspace.clone(),
        ..config_in(&temp)
    };
    drop(owner_state(&sentinel_config));
    drop(queue(
        &sentinel_workspace
            .join("memory")
            .join("sentinel-writes.json"),
    ));

    let owners = OWNERS.lock().unwrap();
    assert!(keys.iter().all(|key| !owners.contains_key(key)));
    drop(owners);
    let queues = QUEUES.lock().unwrap();
    assert!(queue_files.iter().all(|file| !queues.contains_key(file)));
}

#[tokio::test]
async fn restart_drains_scrubbed_learnings_and_ordered_forgetting() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let facts = facts(&config);
    let ack = enqueue(
        &config,
        &json!({"action":"learn","text":"Prefers tea"}),
        &facts,
    )
    .unwrap();
    let id = ack["id"].as_str().unwrap();
    enqueue(&config, &json!({"action":"forget","ids":[id]}), &facts).unwrap();
    // A fresh config and engine sees only the durable file from before restart.
    let restarted = config_in(&tmp);
    let engine = bind_reference(&restarted);
    assert_eq!(drain(&restarted).await, 2);
    assert!(stored(&engine, MetaFilter::default()).await.is_empty());
    assert!(read(&config.workspace_dir).unwrap().is_empty());
}

#[tokio::test]
async fn backend_failure_retains_the_head_and_blocks_later_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let facts = facts(&config);
    enqueue(
        &config,
        &json!({"action":"learn","text":"Prefers tea"}),
        &facts,
    )
    .unwrap();
    enqueue(&config, &json!({"action":"forget","ids":["later"]}), &facts).unwrap();
    RefusingEngine::out_of_credits().bind(&config);
    assert_eq!(drain(&config).await, 0);
    let pending = read(&config.workspace_dir).unwrap();
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].status.as_deref(), Some("INSUFFICIENT_CREDITS"));
    assert!(pending[1].status.is_none());
    bind_reference(&config);
    assert_eq!(drain(&config).await, 2);
}

#[test]
fn retries_ignore_provider_invocation_ids_but_relearning_after_forget_is_ordered() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let mut facts = facts(&config);
    let args = json!({"action":"learn","text":"Prefers tea"});
    facts.tool_call_id = Some("first-provider-id".into());
    let first = enqueue(&config, &args, &facts).unwrap();
    facts.tool_call_id = Some("second-provider-id".into());
    let second = enqueue(&config, &args, &facts).unwrap();
    assert_eq!(first, second);
    assert_eq!(read(&config.workspace_dir).unwrap().len(), 1);
    enqueue(
        &config,
        &json!({"action":"forget","ids":[first["id"]]}),
        &facts,
    )
    .unwrap();
    enqueue(&config, &args, &facts).unwrap();
    assert_eq!(read(&config.workspace_dir).unwrap().len(), 3);
}

#[test]
fn persistence_failure_and_invalid_scope_never_acknowledge_a_write() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let mut facts = facts(&config);
    facts.namespace = "user:someone-else".parse::<Namespace>().unwrap();
    facts.reach = Reach::subtree("user:trusted".parse().unwrap());
    let args = json!({"action":"learn","text":"Prefers tea"});
    assert!(enqueue(&config, &args, &facts).is_err());
    assert!(!path(&config.workspace_dir).exists());
    std::fs::write(
        config.workspace_dir.join("storage"),
        "blocks the storage directory",
    )
    .unwrap();
    assert!(enqueue(&config, &args, &self::facts(&config)).is_err());
}

#[test]
fn private_outbox_is_scrubbed_before_persistence_and_has_a_size_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let secret = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let text = format!("The API token is {secret}");
    enqueue(
        &config,
        &json!({"action":"learn","text":text}),
        &facts(&config),
    )
    .unwrap();
    let persisted = serde_json::to_string(&read(&config.workspace_dir).unwrap()).unwrap();
    assert!(!persisted.contains(secret));
    assert!(
        !path(&config.workspace_dir).exists(),
        "the queue is a storage value, not a file"
    );
    let huge = "x".repeat(MAX_BYTES);
    assert!(enqueue(
        &config,
        &json!({"action":"learn","text":huge}),
        &facts(&config)
    )
    .is_err());
    assert_eq!(read(&config.workspace_dir).unwrap().len(), 1);
}

struct HeldEngine {
    inner: tinymemory_api::conformance::ReferenceEngine,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    held: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl tinymemory_api::MemoryEngine for HeldEngine {
    fn descriptor(&self) -> &tinymemory_api::EngineDescriptor {
        self.inner.descriptor()
    }
    async fn health(&self) -> tinymemory_api::EngineHealth {
        self.inner.health().await
    }
    async fn recall(
        &self,
        req: tinymemory_api::RecallRequest,
    ) -> tinymemory_api::Result<tinymemory_api::RecallAnswer> {
        self.inner.recall(req).await
    }
    async fn fetch(
        &self,
        req: tinymemory_api::FetchRequest,
    ) -> tinymemory_api::Result<tinymemory_api::FetchPage> {
        self.inner.fetch(req).await
    }
    async fn store(&self, item: StoreItem) -> tinymemory_api::Result<tinymemory_api::StoreReceipt> {
        if self.held.load(std::sync::atomic::Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.store(item).await
    }
    async fn forget(
        &self,
        target: tinymemory_api::ForgetTarget,
    ) -> tinymemory_api::Result<tinymemory_api::ForgetReport> {
        self.inner.forget(target).await
    }
    async fn list(
        &self,
        req: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ListPage> {
        self.inner.list(req).await
    }
    async fn export(
        &self,
        req: tinymemory_api::ListRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ExportPage> {
        self.inner.export(req).await
    }
    async fn consolidate(
        &self,
        req: tinymemory_api::ConsolidateRequest,
    ) -> tinymemory_api::Result<tinymemory_api::ConsolidateReceipt> {
        self.inner.consolidate(req).await
    }
}

/// A guard against a hang, not a latency budget: on a loaded machine the held
/// write still acknowledges promptly, so a generous bound never slows a pass.
const TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
async fn held_engine_writes_do_not_delay_acknowledgements_or_hold_the_producer_lock() {
    use tinytools::Tool;
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = Arc::new(HeldEngine {
        inner: tinymemory_api::conformance::ReferenceEngine::new(),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
        held: std::sync::atomic::AtomicBool::new(true),
    });
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    let tool = crate::memory::MemoryTool::new(Arc::new(config.clone()));
    let first = tokio::time::timeout(
        TIMEOUT,
        tool.execute(json!({"action":"learn","text":"First fact"})),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!first.is_error);
    tokio::time::timeout(TIMEOUT, engine.entered.notified())
        .await
        .unwrap();
    let second = tokio::time::timeout(
        TIMEOUT,
        tool.execute(json!({"action":"learn","text":"Second fact"})),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(!second.is_error);
    assert_eq!(read(&config.workspace_dir).unwrap().len(), 2);
    engine
        .held
        .store(false, std::sync::atomic::Ordering::SeqCst);
    engine.release.notify_one();
    drain(&config).await;
    assert_eq!(stored(&engine.inner, MetaFilter::default()).await.len(), 2);
}

#[test]
fn corrupt_existing_outbox_is_preserved_and_refuses_new_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let file = path(&config.workspace_dir);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, b"corrupt existing state").unwrap();
    assert!(enqueue(
        &config,
        &json!({"action":"learn","text":"Prefers tea"}),
        &facts(&config)
    )
    .is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"corrupt existing state");
}

#[tokio::test]
async fn a_different_owner_cannot_replay_the_original_owners_queue() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    enqueue(
        &config,
        &json!({"action":"learn","text":"Prefers tea"}),
        &facts(&config),
    )
    .unwrap();
    {
        let _guard = LOCK.lock().unwrap();
        let mut entries = read(&config.workspace_dir).unwrap();
        entries[0].binding = binding_of(Some("org:another-user".into()), false);
        write(&config.workspace_dir, &entries).unwrap();
    }
    let engine = bind_reference(&config);
    assert_eq!(drain(&config).await, 0);
    assert!(stored(&engine, MetaFilter::default()).await.is_empty());
    assert_eq!(read(&config.workspace_dir).unwrap().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn background_writes_can_complete_after_the_foreground_fifteen_second_budget() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = Arc::new(HeldEngine {
        inner: tinymemory_api::conformance::ReferenceEngine::new(),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
        held: std::sync::atomic::AtomicBool::new(true),
    });
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    let ack = enqueue(
        &config,
        &json!({"action":"learn","text":"Slow but accepted learning"}),
        &facts(&config),
    )
    .unwrap();
    assert_eq!(ack["status"], QUEUED);
    let drain_config = config.clone();
    let draining = tokio::spawn(async move { drain(&drain_config).await });
    engine.entered.notified().await;
    tokio::time::advance(Duration::from_secs(56)).await;
    engine
        .held
        .store(false, std::sync::atomic::Ordering::SeqCst);
    engine.release.notify_one();
    assert_eq!(draining.await.unwrap(), 1);
    assert_eq!(stored(&engine.inner, MetaFilter::default()).await.len(), 1);
}

#[test]
fn a_full_outbox_keeps_existing_writes_and_refuses_a_new_one() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    enqueue(
        &config,
        &json!({"action":"learn","text":"first"}),
        &facts(&config),
    )
    .unwrap();
    let mut entries = read(&config.workspace_dir).unwrap();
    entries.resize(MAX_PENDING, entries[0].clone());
    write(&config.workspace_dir, &entries).unwrap();
    assert!(enqueue(
        &config,
        &json!({"action":"learn","text":"new"}),
        &facts(&config)
    )
    .is_err());
    assert_eq!(read(&config.workspace_dir).unwrap().len(), MAX_PENDING);
}

#[tokio::test]
async fn model_supplied_forget_reach_cannot_remove_another_identitys_item() {
    use tinymemory_api::{LearningKind, MemoryEngine, MemoryMeta};
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let other_facts = CallFacts::of(
        &config,
        &tinymemory_tools::layout::identity::Identity::agent("different-agent"),
    );
    let item = StoreItem::learning(
        "Other identity's learning",
        LearningKind::Fact,
        0.8,
        MemoryMeta {
            namespace: other_facts.namespace,
            ..MemoryMeta::default()
        },
    );
    let id = engine.store(item).await.unwrap().id.0;
    // All agents under one identity share their root by design. Choose an
    // unrelated root explicitly to exercise a foreign identity boundary.
    let mut config = config;
    config.memory.root = Some("user:isolated".into());
    let mut confined = facts(&config);
    confined.reach = Reach::subtree("user:isolated".parse().unwrap());
    let rejected = enqueue(
        &config,
        &json!({"action":"forget","ids":[id],"reach":{"at":"","inherit":true,"descendants":true}}),
        &confined,
    );
    assert!(matches!(
        rejected,
        Err(tinymemory_api::Error::InvalidRequest(_))
    ));
    assert!(read(&config.workspace_dir).unwrap().is_empty());
    enqueue(&config, &json!({"action":"forget","ids":[id]}), &confined).unwrap();
    assert_eq!(drain(&config).await, 1);
    assert_eq!(stored(&engine, MetaFilter::default()).await.len(), 1);
}

#[tokio::test]
async fn explicit_memory_disable_refuses_new_writes_and_keeps_old_writes_pending() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    let engine = bind_reference(&config);
    let call_facts = facts(&config);
    let args = json!({"action":"learn","text":"An earlier accepted learning"});
    enqueue(&config, &args, &call_facts).unwrap();
    config.memory.engine = super::super::engine::DISABLED_ENGINE.to_string();
    assert!(matches!(
        enqueue(
            &config,
            &json!({"action":"learn","text":"Must not persist"}),
            &call_facts
        ),
        Err(Error::Off(_))
    ));
    assert_eq!(drain(&config).await, 0);
    assert_eq!(read(&config.workspace_dir).unwrap().len(), 1);
    assert!(stored(&engine, MetaFilter::default()).await.is_empty());
    config.memory.engine = super::super::engine::TINYHUMANS_ENGINE.to_string();
    assert_eq!(drain(&config).await, 1);
    assert_eq!(stored(&engine, MetaFilter::default()).await.len(), 1);
}

#[tokio::test]
async fn an_erase_fence_clears_prior_pending_learnings_across_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    enqueue(
        &config,
        &json!({"action":"learn","text":"Remove this pending learning"}),
        &facts(&config),
    )
    .unwrap();
    let fence = fence_and_clear(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .await
        .unwrap();
    assert!(read(&config.workspace_dir).unwrap().is_empty());
    drop(fence);
    let restarted = config_in(&tmp);
    let engine = bind_reference(&restarted);
    assert_eq!(drain(&restarted).await, 0);
    assert!(stored(&engine, MetaFilter::default()).await.is_empty());
}

#[tokio::test]
async fn an_erase_fence_waits_for_only_the_current_write_and_preserves_later_enqueues() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = Arc::new(HeldEngine {
        inner: tinymemory_api::conformance::ReferenceEngine::new(),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
        held: std::sync::atomic::AtomicBool::new(true),
    });
    crate::memory::engine::install_test_engine(&config.workspace_dir, engine.clone());
    enqueue(
        &config,
        &json!({"action":"learn","text":"Current write"}),
        &facts(&config),
    )
    .unwrap();
    enqueue(
        &config,
        &json!({"action":"learn","text":"Earlier pending write"}),
        &facts(&config),
    )
    .unwrap();
    schedule(
        &crate::core::runtime::DispatchCtx::ambient(),
        Arc::new(config.clone()),
        &Namespace::ROOT,
    );
    engine.entered.notified().await;
    let fence_config = config.clone();
    let fencing = tokio::spawn(async move {
        fence_and_clear(&crate::core::runtime::DispatchCtx::ambient(), &fence_config).await
    });
    let state = owner_state(&config);
    while !state.gate.is_fenced() {
        tokio::task::yield_now().await;
    }
    assert!(
        !fencing.is_finished(),
        "fence must wait for the held mutation"
    );
    enqueue(
        &config,
        &json!({"action":"learn","text":"Current write"}),
        &facts(&config),
    )
    .unwrap();
    engine
        .held
        .store(false, std::sync::atomic::Ordering::SeqCst);
    engine.release.notify_one();
    let fence = fencing.await.unwrap().unwrap();
    assert_eq!(
        stored(&engine.inner, MetaFilter::default()).await.len(),
        1,
        "the earlier pending write must not be sent"
    );
    enqueue(
        &config,
        &json!({"action":"learn","text":"After erase was requested"}),
        &facts(&config),
    )
    .unwrap();
    schedule(
        &crate::core::runtime::DispatchCtx::ambient(),
        Arc::new(config.clone()),
        &Namespace::ROOT,
    );
    assert_eq!(read(&config.workspace_dir).unwrap().len(), 2);
    drop(fence);
    drain(&config).await;
    assert_eq!(stored(&engine.inner, MetaFilter::default()).await.len(), 2);
}

#[tokio::test]
async fn separate_owner_roots_share_a_workspace_without_blocking_or_erasing_each_other() {
    let tmp = tempfile::tempdir().unwrap();
    let mut first = config_in(&tmp);
    first.memory.root = Some("user:first".into());
    first.config_path = tmp
        .path()
        .join("users/111111111111111111111111/config.toml");
    let mut second = first.clone();
    second.memory.root = Some("user:second".into());
    second.config_path = tmp
        .path()
        .join("users/222222222222222222222222/config.toml");
    let engine = bind_reference(&first);
    enqueue(
        &first,
        &json!({"action":"learn","text":"First owner pending"}),
        &facts(&first),
    )
    .unwrap();
    enqueue(
        &second,
        &json!({"action":"learn","text":"Second owner pending"}),
        &facts(&second),
    )
    .unwrap();
    let fence = fence_and_clear(&crate::core::runtime::DispatchCtx::ambient(), &first)
        .await
        .unwrap();
    assert_eq!(
        drain(&second).await,
        1,
        "another owner's fence cannot block this owner"
    );
    let second_items = stored(&engine, MetaFilter::default()).await;
    assert_eq!(second_items.len(), 1);
    assert!(second_items[0].text.contains("Second owner"));
    assert_eq!(drain(&second).await, 0);
    drop(fence);
    assert_eq!(drain(&first).await, 0);
}

#[tokio::test]
async fn erasing_one_owner_fences_pending_writes_from_every_team_root() {
    let tmp = tempfile::tempdir().unwrap();
    let mut first = config_in(&tmp);
    first.memory.root = Some("team:first".into());
    let mut second = first.clone();
    second.memory.root = Some("team:second".into());
    enqueue(
        &first,
        &json!({"action":"learn","text":"First team pending"}),
        &facts(&first),
    )
    .unwrap();
    enqueue(
        &second,
        &json!({"action":"learn","text":"Second team pending"}),
        &facts(&second),
    )
    .unwrap();
    let fence = fence_and_clear(&crate::core::runtime::DispatchCtx::ambient(), &first)
        .await
        .unwrap();
    assert!(read_at(&queue_path(&first, &facts(&first).reach.at))
        .unwrap()
        .is_empty());
    assert!(read_at(&queue_path(&second, &facts(&second).reach.at))
        .unwrap()
        .is_empty());
    drop(fence);
    let engine = bind_reference(&first);
    assert_eq!(drain(&first).await, 0);
    assert_eq!(drain(&second).await, 0);
    assert!(stored(&engine, MetaFilter::default()).await.is_empty());
}

#[tokio::test]
async fn a_later_erase_clears_pending_writes_admitted_during_the_first_erase() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let first = fence_and_clear(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .await
        .unwrap();
    enqueue(
        &config,
        &json!({"action":"learn","text":"Queued after the first erase began"}),
        &facts(&config),
    )
    .unwrap();
    drop(first);
    let second = fence_and_clear(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .await
        .unwrap();
    assert!(
        read(&config.workspace_dir).unwrap().is_empty(),
        "the second erase must discard a learning admitted before it started"
    );
    drop(second);
}

#[test]
fn an_outbox_written_before_the_binding_field_is_still_read() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let file = path(&config.workspace_dir);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let legacy = serde_json::json!([{
        "ticket": "t-1",
        "root": null,
        "layout_v3": false,
        "scope_root": "",
        "epoch": ["old-process", 3],
        "operation": {"action": "forget", "ids": ["a"], "reach": serde_json::to_value(Reach::subtree(Namespace::ROOT)).unwrap()},
        "status": "TIMEOUT"
    }]);
    std::fs::write(&file, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let entries = read(&config.workspace_dir).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].binding, binding_of(None, false));
    assert_eq!(entries[0].status.as_deref(), Some("TIMEOUT"));
    assert_eq!(entries[0].epoch.as_ref().unwrap().generation, 3);
}
