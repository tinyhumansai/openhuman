//! Wake-graph construction and execution.
//!
//! Holds the injected [`OrchestrationRuntime`] seam, the reducer + node wiring
//! ([`build_orchestration_graph`]), the durable run entrypoint
//! ([`run_orchestration_graph`]), and the structure-only topology export
//! ([`orchestration_graph_topology`]). See the [module docs](super) for the
//! graph shape and routing rules.

use std::sync::Arc;

use async_trait::async_trait;
use tinyagents::graph::export::GraphTopology;
use tinyagents::graph::{
    ClosureStateReducer, Command, CompiledGraph, GraphBuilder, NodeContext, NodeResult,
};

use super::state::{CompressedEntry, OrchestrationState, WorldDiffEntry};
use crate::openhuman::config::Config;
use crate::openhuman::tinyagents::observability::GraphTracingSink;
use tinyagents::graph::SqliteCheckpointer;

const LOG: &str = "orchestration";

/// The reasoning core's output for one cycle.
pub struct ExecuteOutcome {
    /// The answer the front end compiles into a channel reply on pass 2.
    pub reply: String,
    /// Raw execution trace (assistant text + tool/sub-agent activity) that the
    /// `compress` node condenses 20:1.
    pub trace: String,
}

/// Result of the `context_guard` eviction pass.
pub struct EvictionOutcome {
    /// How many oldest compressed-history entries were pushed to memory + dropped.
    pub evicted: usize,
    /// Utilization (0.0–1.0) after eviction.
    pub new_utilization: f32,
}

/// Every behaviour-bearing operation of the wake graph, injected as one trait so
/// the graph structure is hermetically testable with a single stub.
#[async_trait]
pub trait OrchestrationRuntime: Send + Sync {
    /// Front-end pass 1 — raw traffic → macro-instructions for the reasoning core.
    async fn frontend_instruct(&self, state: &OrchestrationState) -> anyhow::Result<String>;
    /// Front-end pass 2 — reasoning reply → finished channel-response text.
    async fn frontend_compile(&self, state: &OrchestrationState) -> anyhow::Result<String>;
    /// Reasoning core — applies steering, spawns sub-agents, returns reply + trace.
    async fn execute(&self, state: &OrchestrationState) -> anyhow::Result<ExecuteOutcome>;
    /// 20:1-compress the cycle's execution trace and persist a store row.
    async fn compress(&self, state: &OrchestrationState) -> anyhow::Result<CompressedEntry>;
    /// Append one world-state-diff timeline entry (store-persisted, append-only).
    async fn world_diff(&self, state: &OrchestrationState) -> anyhow::Result<WorldDiffEntry>;
    /// Context-window utilization (0.0–1.0) for this cycle's accumulated state.
    async fn context_utilization(&self, state: &OrchestrationState) -> anyhow::Result<f32>;
    /// Evict the oldest compressed-history entries to memory RAG.
    async fn evict(&self, state: &OrchestrationState) -> anyhow::Result<EvictionOutcome>;
    /// Send the compiled `channel_response` back over the tiny.place DM.
    async fn send_dm(&self, counterpart_agent_id: &str, body: &str) -> anyhow::Result<()>;
}

/// Reducer update emitted by an orchestration node. Exactly one per node result
/// (the crate applies a single `Update` per boundary). Public only because it
/// appears in the [`CompiledGraph`] type parameter [`build_orchestration_graph`]
/// returns; the variants are an internal reducer detail.
pub enum OrchestrationUpdate {
    /// Front-end pass 1: store macro-instructions + advance the pass counter.
    Pass1 { instructions: String },
    /// Front-end pass 2: store the finished channel response + advance the pass
    /// counter. Its presence is the terminate predicate.
    Pass2 { channel_response: String },
    /// Reasoning core produced a reply + trace.
    Executed { reply: String, trace: String },
    /// Compression node appended a compressed-history entry.
    PushCompressed(CompressedEntry),
    /// World-diff node appended a timeline entry.
    PushWorldDiff(WorldDiffEntry),
    /// Context guard measured utilization (no eviction).
    Context(f32),
    /// Context guard evicted `count` oldest entries and reset utilization.
    Evicted { count: usize, utilization: f32 },
    /// The outbound DM was dispatched — latch so it can never double-send.
    DmSent,
    /// No state change (structural nodes).
    Noop,
}

/// Lift an injected node's `anyhow` error into the graph error type.
fn graph_err(e: anyhow::Error) -> tinyagents::TinyAgentsError {
    tinyagents::TinyAgentsError::Graph(e.to_string())
}

/// Build (but do not run) the orchestration wake graph. Shared by
/// [`run_orchestration_graph`] and [`orchestration_graph_topology`].
///
/// `max_supersteps` is the loop-continuity backstop; `evict_threshold` is the
/// context-guard eviction trigger (clamped 0.8–0.9 by the caller).
pub fn build_orchestration_graph(
    runtime: Arc<dyn OrchestrationRuntime>,
    max_supersteps: u32,
    evict_threshold: f32,
) -> anyhow::Result<CompiledGraph<OrchestrationState, OrchestrationUpdate>> {
    let mut builder = GraphBuilder::<OrchestrationState, OrchestrationUpdate>::new().set_reducer(
        ClosureStateReducer::new(|mut s: OrchestrationState, u: OrchestrationUpdate| {
            match u {
                OrchestrationUpdate::Pass1 { instructions } => {
                    s.agent_instructions = Some(instructions);
                    s.pass += 1;
                }
                OrchestrationUpdate::Pass2 { channel_response } => {
                    s.channel_response = Some(channel_response);
                    s.pass += 1;
                }
                OrchestrationUpdate::Executed { reply, trace } => {
                    s.agent_reply = Some(reply);
                    s.execution_trace = trace;
                }
                OrchestrationUpdate::PushCompressed(entry) => s.compressed_history.push(entry),
                OrchestrationUpdate::PushWorldDiff(entry) => s.world_state_diff.entries.push(entry),
                OrchestrationUpdate::Context(util) => s.context_utilization = util,
                OrchestrationUpdate::Evicted { count, utilization } => {
                    let drop = count.min(s.compressed_history.len());
                    s.compressed_history.drain(0..drop);
                    s.context_utilization = utilization;
                }
                OrchestrationUpdate::DmSent => s.dm_sent = true,
                OrchestrationUpdate::Noop => {}
            }
            Ok(s)
        }),
    );

    // `normalize`: window already folded into state before the run (ops::seed_state).
    builder = builder.add_node(
        "normalize",
        |s: OrchestrationState, _c: NodeContext| async move {
            tracing::debug!(
                target: LOG, session_id = %s.session_id, cycle_id = %s.cycle_id,
                node = "normalize", messages = s.messages.len(), "[orchestration] node.enter",
            );
            Ok(NodeResult::Update(OrchestrationUpdate::Noop))
        },
    );

    // `frontend`: the router. Two-pass, Quick LLM, conditional goto.
    {
        let runtime = runtime.clone();
        builder = builder.add_node("frontend", move |s: OrchestrationState, _c: NodeContext| {
            let runtime = runtime.clone();
            async move {
                let pass = s.pass + 1;
                // Local Master chat (human ↔ OpenHuman itself): the A2A front-end
                // triage does NOT run — the human talks straight to the reasoning
                // core (OpenHuman). We feed the core a direct human-facing directive
                // on the way in and use its answer verbatim on the way out.
                let is_local_master = s.counterpart_agent_id
                    == crate::openhuman::orchestration::types::LOCAL_MASTER_AGENT;

                // Defensive terminate: a response already exists (re-entry / resume).
                if s.channel_response.is_some() {
                    tracing::debug!(
                        target: LOG, session_id = %s.session_id, node = "frontend", pass,
                        route = "send_dm", reason = "channel_response_present",
                        "[orchestration] node.route",
                    );
                    return Ok(NodeResult::Command(
                        Command::default().with_goto(["send_dm"]),
                    ));
                }

                // Loop-continuity backstop (spec §5): never cycle past the cap.
                if pass > max_supersteps {
                    let body = s.agent_reply.clone().unwrap_or_else(|| "…".to_string());
                    tracing::warn!(
                        target: LOG, session_id = %s.session_id, node = "frontend", pass,
                        route = "send_dm", reason = "max_supersteps_backstop",
                        "[orchestration] node.route",
                    );
                    return Ok(NodeResult::Command(
                        Command::default()
                            .with_update(OrchestrationUpdate::Pass2 {
                                channel_response: body,
                            })
                            .with_goto(["send_dm"]),
                    ));
                }

                // Pass 2: reasoning replied → compile the channel response. For a
                // local master cycle, skip the A2A "compile" and use the core's
                // answer verbatim (it already phrased it for the human).
                if s.agent_reply.is_some() {
                    let body = if is_local_master {
                        s.agent_reply.clone().unwrap_or_default()
                    } else {
                        runtime.frontend_compile(&s).await.map_err(graph_err)?
                    };
                    tracing::debug!(
                        target: LOG, session_id = %s.session_id, node = "frontend", pass,
                        route = "send_dm", reason = "reply_ready", local = is_local_master,
                        "[orchestration] node.route",
                    );
                    return Ok(NodeResult::Command(
                        Command::default()
                            .with_update(OrchestrationUpdate::Pass2 {
                                channel_response: body,
                            })
                            .with_goto(["send_dm"]),
                    ));
                }

                // Pass 1: hand down to the core. For a local master cycle there is
                // no A2A front-end triage — the `execute` node runs `master_agent`
                // (its human-facing system prompt carries the framing + tool guide),
                // so we only need a light directive here, not `frontend_instruct`.
                let instructions = if is_local_master {
                    "Answer your human's latest message in the conversation below.".to_string()
                } else {
                    runtime.frontend_instruct(&s).await.map_err(graph_err)?
                };
                tracing::debug!(
                    target: LOG, session_id = %s.session_id, node = "frontend", pass,
                    route = "execute", reason = "first_pass", local = is_local_master,
                    "[orchestration] node.route",
                );
                Ok(NodeResult::Command(
                    Command::default()
                        .with_update(OrchestrationUpdate::Pass1 { instructions })
                        .with_goto(["execute"]),
                ))
            }
        });
    }

    // `execute`: reasoning core — applies steering, spawns sub-agents, sets reply.
    {
        let runtime = runtime.clone();
        builder = builder.add_node("execute", move |s: OrchestrationState, _c: NodeContext| {
            let runtime = runtime.clone();
            async move {
                let out = runtime.execute(&s).await.map_err(graph_err)?;
                tracing::debug!(
                    target: LOG, session_id = %s.session_id, cycle_id = %s.cycle_id,
                    node = "execute", trace_len = out.trace.len(), "[orchestration] node.exit",
                );
                Ok(NodeResult::Update(OrchestrationUpdate::Executed {
                    reply: out.reply,
                    trace: out.trace,
                }))
            }
        });
    }

    // `compress`: 20:1-compress the cycle trace, persist a compressed-history row.
    {
        let runtime = runtime.clone();
        builder = builder.add_node("compress", move |s: OrchestrationState, _c: NodeContext| {
            let runtime = runtime.clone();
            async move {
                let entry = runtime.compress(&s).await.map_err(graph_err)?;
                tracing::debug!(
                    target: LOG, session_id = %s.session_id, cycle_id = %s.cycle_id,
                    node = "compress", covered = entry.covered_messages, "[orchestration] node.exit",
                );
                Ok(NodeResult::Update(OrchestrationUpdate::PushCompressed(entry)))
            }
        });
    }

    // `world_diff`: append one append-only timeline entry, persist a store row.
    {
        let runtime = runtime.clone();
        builder = builder.add_node(
            "world_diff",
            move |s: OrchestrationState, _c: NodeContext| {
                let runtime = runtime.clone();
                async move {
                    let entry = runtime.world_diff(&s).await.map_err(graph_err)?;
                    tracing::debug!(
                        target: LOG, session_id = %s.session_id, cycle_id = %s.cycle_id,
                        node = "world_diff", seq = entry.seq, "[orchestration] node.exit",
                    );
                    Ok(NodeResult::Update(OrchestrationUpdate::PushWorldDiff(
                        entry,
                    )))
                }
            },
        );
    }

    // `send_dm`: the outbound Signal reply. Sends at most once (dm_sent latch).
    {
        let runtime = runtime.clone();
        builder = builder.add_node("send_dm", move |s: OrchestrationState, _c: NodeContext| {
            let runtime = runtime.clone();
            async move {
                if s.dm_sent {
                    tracing::debug!(
                        target: LOG, session_id = %s.session_id, node = "send_dm",
                        reason = "already_sent", "[orchestration] node.skip",
                    );
                } else if let Some(body) = s.channel_response.as_deref() {
                    runtime
                        .send_dm(&s.counterpart_agent_id, body)
                        .await
                        .map_err(graph_err)?;
                    tracing::debug!(
                        target: LOG, session_id = %s.session_id, node = "send_dm",
                        counterpart = %s.counterpart_agent_id, "[orchestration] node.sent",
                    );
                }
                Ok(NodeResult::Update(OrchestrationUpdate::DmSent))
            }
        });
    }

    // `context_guard`: utilization + eviction. Runs after all mutations, before END.
    {
        let runtime = runtime.clone();
        builder = builder.add_node(
            "context_guard",
            move |s: OrchestrationState, _c: NodeContext| {
                let runtime = runtime.clone();
                async move {
                    let util = runtime.context_utilization(&s).await.map_err(graph_err)?;
                    if util >= evict_threshold {
                        let ev = runtime.evict(&s).await.map_err(graph_err)?;
                        tracing::debug!(
                            target: LOG, session_id = %s.session_id, node = "context_guard",
                            utilization = util, evicted = ev.evicted,
                            new_utilization = ev.new_utilization, "[orchestration] node.evict",
                        );
                        Ok(NodeResult::Update(OrchestrationUpdate::Evicted {
                            count: ev.evicted,
                            utilization: ev.new_utilization,
                        }))
                    } else {
                        tracing::debug!(
                            target: LOG, session_id = %s.session_id, node = "context_guard",
                            utilization = util, "[orchestration] node.exit",
                        );
                        Ok(NodeResult::Update(OrchestrationUpdate::Context(util)))
                    }
                }
            },
        );
    }

    let graph = builder
        .add_node(
            "done",
            |_s: OrchestrationState, _c: NodeContext| async move {
                Ok(NodeResult::Update(OrchestrationUpdate::Noop))
            },
        )
        .add_edge("normalize", "frontend")
        .add_edge("execute", "compress")
        .add_edge("compress", "world_diff")
        .add_edge("world_diff", "frontend")
        .add_edge("send_dm", "context_guard")
        .add_edge("context_guard", "done")
        .set_entry("normalize")
        .mark_command_routing("frontend")
        .set_finish("done")
        .compile()
        .map_err(|e| anyhow::anyhow!("orchestration graph compile failed: {e}"))?;
    Ok(graph)
}

/// Drive one wake cycle for `state.session_id`, checkpointing every super-step
/// boundary under thread `orchestration:<counterpart>:<session_id>`. Returns the
/// terminal state.
pub async fn run_orchestration_graph(
    config: Arc<Config>,
    runtime: Arc<dyn OrchestrationRuntime>,
    state: OrchestrationState,
) -> anyhow::Result<OrchestrationState> {
    let max = config.orchestration.max_supersteps;
    let threshold = config.orchestration.effective_evict_threshold();
    // Scope the checkpoint thread by (counterpart, session) — the same key the
    // store uses for sessions — so two peers that share a session id (a legacy
    // `harness_session_id` fallback collision) never resume from each other's
    // checkpoint. Migration seam: a cycle checkpointed under the old
    // `orchestration:<session_id>` key that only resumes AFTER this upgrade starts
    // a fresh thread — one extra in-flight cycle at the boundary (Beta, gated by
    // `[orchestration]`), the same worst case as the `cycle_id` format change.
    let thread_id = format!(
        "orchestration:{}:{}",
        state.counterpart_agent_id, state.session_id
    );
    let label = thread_id.clone();
    // `SqlRunLedgerCheckpointer` was retired in favor of the crate's own
    // `SqliteCheckpointer` (see `agent_orchestration/delegation.rs`); mirrors
    // that swap here with a dedicated `orchestration_graph_checkpoints.db`.
    let checkpoint_db = config
        .workspace_dir
        .join("orchestration_graph_checkpoints.db");
    let checkpointer = Arc::new(
        SqliteCheckpointer::<OrchestrationState>::open(&checkpoint_db)
            .map_err(|e| anyhow::anyhow!("open durable orchestration checkpoint store: {e}"))?,
    );

    tracing::debug!(
        target: LOG, session_id = %state.session_id, %thread_id,
        messages = state.messages.len(), "[orchestration] graph.run.enter",
    );

    let graph = build_orchestration_graph(runtime, max, threshold)?
        .with_checkpointer(checkpointer)
        .with_event_sink(Arc::new(GraphTracingSink::new(label)));

    let exec = graph
        .run_with_thread(thread_id, state)
        .await
        .map_err(|e| anyhow::anyhow!("orchestration graph run failed: {e}"))?;

    tracing::debug!(
        target: LOG, session_id = %exec.state.session_id, steps = exec.steps,
        dm_sent = exec.state.dm_sent, pass = exec.state.pass,
        compressed = exec.state.compressed_history.len(),
        diff_entries = exec.state.world_state_diff.entries.len(),
        "[orchestration] graph.run.exit",
    );
    Ok(exec.state)
}

/// Structure-only [`GraphTopology`] of the wake graph for debug / inspection.
/// Built with a no-op runtime — exposes only node names, edges, and routing.
pub fn orchestration_graph_topology() -> anyhow::Result<GraphTopology> {
    struct NoopRuntime;
    #[async_trait]
    impl OrchestrationRuntime for NoopRuntime {
        async fn frontend_instruct(&self, _s: &OrchestrationState) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn frontend_compile(&self, _s: &OrchestrationState) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn execute(&self, _s: &OrchestrationState) -> anyhow::Result<ExecuteOutcome> {
            Ok(ExecuteOutcome {
                reply: String::new(),
                trace: String::new(),
            })
        }
        async fn compress(&self, _s: &OrchestrationState) -> anyhow::Result<CompressedEntry> {
            Ok(CompressedEntry::default())
        }
        async fn world_diff(&self, _s: &OrchestrationState) -> anyhow::Result<WorldDiffEntry> {
            Ok(WorldDiffEntry::default())
        }
        async fn context_utilization(&self, _s: &OrchestrationState) -> anyhow::Result<f32> {
            Ok(0.0)
        }
        async fn evict(&self, _s: &OrchestrationState) -> anyhow::Result<EvictionOutcome> {
            Ok(EvictionOutcome {
                evicted: 0,
                new_utilization: 0.0,
            })
        }
        async fn send_dm(&self, _c: &str, _b: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    let graph = build_orchestration_graph(Arc::new(NoopRuntime), 12, 0.85)?;
    Ok(graph.topology())
}
