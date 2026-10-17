//! Removed agents cannot leave approval callbacks or polling handles attached to reused IDs.
mod common;

use async_trait::async_trait;
use openhuman_embed::{
    Access, AgentDefinitionSpec, AgentError, AgentSpec, AgentTurnOrigin, ApprovalDecision,
    ApprovalHandler, ApprovalsError, HostTurnTools, PendingApproval, Runtime, Tool, ToolScopeSpec,
    TrustedAccess,
};
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Condvar, Mutex,
};
use std::time::Duration;
use tokio::sync::mpsc;

struct ApproveAndRecord {
    label: &'static str,
    calls: Arc<AtomicUsize>,
    seen: mpsc::UnboundedSender<&'static str>,
}
#[async_trait]
impl ApprovalHandler for ApproveAndRecord {
    async fn decide(&self, _: &PendingApproval) -> ApprovalDecision {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let _ = self.seen.send(self.label);
        ApprovalDecision::ApproveOnce
    }
}

struct WaitForRemoval {
    label: &'static str,
    started: mpsc::UnboundedSender<(&'static str, String)>,
    dropped: mpsc::UnboundedSender<&'static str>,
}
struct DecisionDrop {
    label: &'static str,
    dropped: mpsc::UnboundedSender<&'static str>,
}
impl Drop for DecisionDrop {
    fn drop(&mut self) {
        let _ = self.dropped.send(self.label);
    }
}
#[async_trait]
impl ApprovalHandler for WaitForRemoval {
    async fn decide(&self, request: &PendingApproval) -> ApprovalDecision {
        let _guard = DecisionDrop {
            label: self.label,
            dropped: self.dropped.clone(),
        };
        let _ = self.started.send((self.label, request.request_id.clone()));
        std::future::pending().await
    }
}

fn supervised(id: &str, scratch: &Path, provider: &wiremock::MockServer) -> AgentSpec {
    AgentSpec::new(id)
        .provider(common::route(provider, "approval-fixture"))
        .action_dir(scratch)
        .access(
            Access::supervised()
                .origin(AgentTurnOrigin::WebChat {
                    thread_id: id.into(),
                    client_id: "lifecycle-test".into(),
                    request_id: None,
                })
                .auto_approve(Vec::<String>::new())
                .auto_approve_all(false)
                .trust(scratch.display().to_string(), TrustedAccess::ReadWrite),
        )
        .definition(AgentDefinitionSpec::new().tools(ToolScopeSpec::Named(vec!["shell".into()])))
}

async fn writing_provider(marker: &Path) -> wiremock::MockServer {
    common::scripted_provider(
        vec![common::tool_call_completion(
            "shell",
            &serde_json::json!({"command": format!("touch {}", marker.display())}).to_string(),
        )],
        "denied without writing",
    )
    .await
}

#[test]
fn removal_invalidates_approval_handles_and_reserves_ids_until_cleanup() {
    common::runtime()
        .block_on(async {
            tokio::spawn(async {
            let backend = common::stub_backend().await;
            let provider = common::provider("no tools").await;
            let scratch = tempfile::tempdir().unwrap();
            let mut config = common::offline_config();
            config.autonomy.enabled = true;
            let runtime = Arc::new(Runtime::builder()
                .config(config)
                .backend_url(backend.uri())
                .provider(common::route(&provider, "approval-fixture"))
                .build()
                .await
                .unwrap());
            let builder_calls = Arc::new(AtomicUsize::new(0));
            let live_calls = Arc::new(AtomicUsize::new(0));
            let (old_seen, mut old_seen_rx) = mpsc::unbounded_channel();
            let original = runtime
                .agent(supervised("reused", scratch.path(), &provider).approval_handler(Arc::new(
                    ApproveAndRecord {
                        label: "removed builder callback",
                        calls: builder_calls.clone(),
                        seen: old_seen.clone(),
                    },
                )))
                .unwrap();
            let original_polling = original.approvals();
            let original_subscription = original.handle_approvals(Arc::new(ApproveAndRecord {
                label: "removed live callback",
                calls: live_calls.clone(),
                seen: old_seen,
            }));
            runtime.remove_agent("reused").await.unwrap();
            assert!(original_polling.pending().unwrap().is_empty());

            let marker = scratch.path().join("replacement-must-not-write");
            let replacement_provider = writing_provider(&marker).await;
            let (started, mut started_rx) = mpsc::unbounded_channel();
            let (dropped, mut dropped_rx) = mpsc::unbounded_channel();
            let replacement = runtime
                .agent(
                    supervised("reused", scratch.path(), &replacement_provider).approval_handler(
                        Arc::new(WaitForRemoval {
                            label: "replacement",
                            started,
                            dropped,
                        }),
                    ),
                )
                .unwrap();
            let turning = replacement.clone();
            let turn = tokio::spawn(async move { turning.run("Create a marker").await });
            let request_id = tokio::time::timeout(Duration::from_secs(10), async {
                tokio::select! {
                    biased;
                    Some(old) = old_seen_rx.recv() => panic!("removed callback observed replacement approval: {old:?}"),
                    request = started_rx.recv() => request.expect("replacement callback starts").1,
                }
            })
            .await
            .expect("replacement approval reaches its callback");
            assert!(!marker.exists(), "replacement write remains parked");
            assert_eq!(builder_calls.load(Ordering::SeqCst), 0);
            assert_eq!(live_calls.load(Ordering::SeqCst), 0);
            assert!(
                original_polling.pending().unwrap().is_empty(),
                "removed polling handle must not see the replacement's request"
            );
            assert!(matches!(
                original_polling.decide(&request_id, ApprovalDecision::ApproveOnce),
                Err(ApprovalsError::NotFound(_))
            ));
            assert!(replacement
                .approvals()
                .pending()
                .unwrap()
                .iter()
                .any(|request| request.request_id == request_id));
            replacement
                .approvals()
                .decide(&request_id, ApprovalDecision::Deny)
                .unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(10), turn)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
                    .reply,
                "denied without writing"
            );
            runtime.remove_agent("reused").await.unwrap();
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(10), dropped_rx.recv())
                    .await
                    .unwrap(),
                Some("replacement")
            );
            assert!(!marker.exists());
            assert_eq!(builder_calls.load(Ordering::SeqCst), 0);
            assert_eq!(live_calls.load(Ordering::SeqCst), 0);

            // Removal also destroys callbacks already waiting on an original
            // request, while both the agent and returned subscription remain live.
            let marker = scratch.path().join("inflight-must-not-write");
            let inflight_provider = writing_provider(&marker).await;
            let (started, mut started_rx) = mpsc::unbounded_channel();
            let (dropped, mut dropped_rx) = mpsc::unbounded_channel();
            let inflight = runtime
                .agent(
                    supervised("inflight", scratch.path(), &inflight_provider).approval_handler(
                        Arc::new(WaitForRemoval {
                            label: "builder",
                            started: started.clone(),
                            dropped: dropped.clone(),
                        }),
                    ),
                )
                .unwrap();
            let inflight_subscription = inflight.handle_approvals(Arc::new(WaitForRemoval {
                label: "live",
                started,
                dropped,
            }));
            let turning = inflight.clone();
            let turn = tokio::spawn(async move { turning.run("Create a marker").await });
            let mut callbacks = BTreeSet::new();
            for _ in 0..2 {
                let (label, _) = tokio::time::timeout(Duration::from_secs(10), started_rx.recv())
                    .await
                    .unwrap()
                    .expect("callback starts");
                callbacks.insert(label);
            }
            assert_eq!(callbacks, BTreeSet::from(["builder", "live"]));
            runtime.remove_agent("inflight").await.unwrap();
            let mut destroyed = BTreeSet::new();
            for _ in 0..2 {
                destroyed.insert(
                    tokio::time::timeout(Duration::from_secs(10), dropped_rx.recv())
                        .await
                        .expect("agent removal drops the pending decision future")
                        .unwrap(),
                );
            }
            assert_eq!(destroyed, callbacks);
            let _ = tokio::time::timeout(Duration::from_secs(10), turn)
                .await
                .unwrap()
                .unwrap();
            assert!(inflight.approvals().pending().unwrap().is_empty());
            assert!(!marker.exists());
            drop(inflight_subscription);
            drop(inflight);
            drop(replacement);
            drop(original_subscription);
            drop(original);
            removal_reserves_ids_until_cleanup_finishes(runtime.clone(), scratch.path()).await;
            natural_drop_reserves_id_until_state_teardown_finishes(runtime.clone()).await;
            drop(runtime);
            })
            .await
            .expect("approval lifecycle worker did not panic");
        });
}

#[derive(Default)]
struct DropBarrier {
    released: Mutex<bool>,
    changed: Condvar,
}
impl DropBarrier {
    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.changed.notify_all();
    }
    fn wait(&self) {
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.changed.wait(released).unwrap();
        }
    }
}
// Always unblock a worker when an assertion unwinds the fixture.
struct ReleaseBarrier(Arc<DropBarrier>);
impl Drop for ReleaseBarrier {
    fn drop(&mut self) {
        self.0.release();
    }
}
struct BlockingDecisionDrop {
    barrier: Arc<DropBarrier>,
    dropping: mpsc::UnboundedSender<()>,
}
impl Drop for BlockingDecisionDrop {
    fn drop(&mut self) {
        let _ = self.dropping.send(());
        // Hand the worker queue to another worker while holding teardown,
        // so the test can observe the destructor-start signal.
        tokio::task::block_in_place(|| self.barrier.wait());
    }
}
struct RemovalTool {
    barrier: Arc<DropBarrier>,
    started: mpsc::UnboundedSender<()>,
    dropping: mpsc::UnboundedSender<()>,
}
#[async_trait]
impl Tool for RemovalTool {
    fn name(&self) -> &str {
        "wait_for_removal"
    }
    fn description(&self) -> &str {
        "A host-owned fixture held until the agent is removed"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object", "properties": {}})
    }
    async fn execute(&self, _: serde_json::Value) -> anyhow::Result<tinytools::ToolResult> {
        let _guard = BlockingDecisionDrop {
            barrier: self.barrier.clone(),
            dropping: self.dropping.clone(),
        };
        let _ = self.started.send(());
        std::future::pending().await
    }
}

async fn removal_reserves_ids_until_cleanup_finishes(runtime: Arc<Runtime>, scratch: &Path) {
    for cancel in [false, true] {
        let id = if cancel {
            "cancelled-removal"
        } else {
            "concurrent-removal"
        };
        let provider = common::scripted_provider(
            vec![common::tool_call_completion("wait_for_removal", "{}")],
            "unreachable",
        )
        .await;
        let barrier = Arc::new(DropBarrier::default());
        let release_on_unwind = ReleaseBarrier(barrier.clone());
        let (started, mut started_rx) = mpsc::unbounded_channel();
        let (dropping, mut dropping_rx) = mpsc::unbounded_channel();
        let tool_barrier = barrier.clone();
        let original = runtime
            .agent(
                AgentSpec::new(id)
                    .provider(common::route(&provider, "removal-fixture"))
                    .action_dir(scratch)
                    .definition(
                        AgentDefinitionSpec::new()
                            .bare_prompt("Use wait_for_removal.")
                            .tools(ToolScopeSpec::HostOnly),
                    )
                    .tools(move |_| {
                        HostTurnTools::advertised(vec![Box::new(RemovalTool {
                            barrier: tool_barrier.clone(),
                            started: started.clone(),
                            dropping: dropping.clone(),
                        })])
                    }),
            )
            .unwrap();
        let home = original.home_dir().to_path_buf();
        let turning = original.clone();
        let turn = tokio::spawn(async move { turning.run("Wait until removed").await });
        tokio::time::timeout(Duration::from_secs(10), started_rx.recv())
            .await
            .unwrap()
            .unwrap();
        let removing_runtime = runtime.clone();
        let removing = tokio::spawn(async move { removing_runtime.remove_agent(id).purge().await });
        // The turn's destructor has begun but cannot release its in-flight
        // accounting yet, so removal is deterministically waiting for idle.
        tokio::time::timeout(Duration::from_secs(10), dropping_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            runtime.agent_ids().iter().any(|reserved| reserved == id),
            "removal must retain the id reservation until cleanup finishes"
        );
        assert!(
            openhuman_core::agent::host_agents::resolve(id).is_none(),
            "removed agents must not admit new sessions through core drivers"
        );
        assert!(
            matches!(runtime.remove_agent(id).await, Err(AgentError::UnknownId(ref removed)) if removed == id),
            "a second removal cannot claim the same cleanup reservation"
        );
        let replacement = runtime.agent(AgentSpec::new(id));
        assert!(
            matches!(replacement, Err(AgentError::DuplicateId(ref duplicate)) if duplicate == id),
            "the id must remain reserved while removal waits for a turn to unwind"
        );
        if cancel {
            removing.abort();
            assert!(removing.await.unwrap_err().is_cancelled());
            // Cancellation transfers the purge to the close task. It must not
            // delete module-owned state before the module acknowledges close.
            tokio::time::timeout(Duration::from_secs(10), async {
                while home.exists() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .expect("deferred purge follows module close acknowledgment");
            barrier.release();
        } else {
            barrier.release();
            removing.await.unwrap().unwrap();
            assert!(
                !home.exists(),
                "normal removal performs purge before releasing the id"
            );
        }
        tokio::time::timeout(Duration::from_secs(10), turn)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        // Keep the original handle live: cancellation must clean up now,
        // rather than relying on its eventual destructor.
        let replacement = runtime
            .agent(AgentSpec::new(id))
            .expect("cleanup releases the reserved id");
        assert!(original.approvals().pending().unwrap().is_empty());
        runtime.remove_agent(id).await.unwrap();
        drop(replacement);
        drop(original);
        drop(release_on_unwind);
    }
}

#[derive(Default)]
struct BlockingStateSlot(Mutex<Option<BlockingDecisionDrop>>);

async fn natural_drop_reserves_id_until_state_teardown_finishes(runtime: Arc<Runtime>) {
    let id = "natural-drop";
    let original = runtime.agent(AgentSpec::new(id)).unwrap();
    let context = openhuman_core::core::runtime::AgentContextRegistry::get(id).unwrap();
    let barrier = Arc::new(DropBarrier::default());
    let release_on_unwind = ReleaseBarrier(barrier.clone());
    let (dropping, mut dropping_rx) = mpsc::unbounded_channel();
    let slot = context.agent_state().slot::<BlockingStateSlot>();
    *slot.0.lock().unwrap() = Some(BlockingDecisionDrop {
        barrier: barrier.clone(),
        dropping,
    });
    drop(slot);
    let dropping_agent = tokio::task::spawn_blocking(move || drop(original));
    tokio::time::timeout(Duration::from_secs(10), dropping_rx.recv())
        .await
        .unwrap()
        .unwrap();
    // The last Agent Arc is already gone, but its destructor is still
    // clearing old-instance state before evicting the keyed MCP host.
    assert!(
        runtime.agent_ids().iter().any(|reserved| reserved == id),
        "the natural destructor keeps its id reserved until teardown finishes"
    );
    assert!(
        matches!(runtime.agent(AgentSpec::new(id)), Err(AgentError::DuplicateId(ref duplicate)) if duplicate == id)
    );
    barrier.release();
    dropping_agent.await.unwrap();
    assert!(context.agent_state().is_empty());
    let replacement = runtime
        .agent(AgentSpec::new(id))
        .expect("natural teardown releases its id");
    // Later release of a retained old context cannot unregister the new one.
    drop(context);
    assert!(openhuman_core::core::runtime::AgentContextRegistry::get(id).is_some());
    runtime.remove_agent(id).await.unwrap();
    drop(replacement);
    drop(release_on_unwind);
}
