//! OpenHuman's RPC and event adapter for TinyAgents' per-thread run-mode store.

use crate::core::runtime::DispatchCtx;
use std::sync::Arc;
use tinyagents_harness::middleware::{RunMode, RunModeHandle};
use tinyagents_harness_tools::run_mode::{InMemoryRunModeStore, ModeChangeNotifier, RunModeStore};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;

struct ThreadRunModes(InMemoryRunModeStore<()>);

struct DomainEventNotifier;

impl ModeChangeNotifier<()> for DomainEventNotifier {
    fn changed(&self, _scope: &(), thread_id: &str, mode: RunMode) {
        BUS.publish(DomainEvent::ThreadRunModeChanged {
            thread_id: thread_id.to_owned(),
            mode: mode_label(mode).to_owned(),
        });
    }
}

impl Default for ThreadRunModes {
    fn default() -> Self {
        Self(InMemoryRunModeStore::new(Arc::new(DomainEventNotifier)))
    }
}

fn registry(ctx: &DispatchCtx) -> Arc<ThreadRunModes> {
    crate::core::runtime::slot_for::<ThreadRunModes>(ctx)
}

/// Returns the [`RunModeHandle`] for `thread_id`, creating one (starting in
/// [`RunMode::Build`]) if none exists yet. Cloning a `RunModeHandle` shares
/// the same underlying atomic, so every clone (a live turn's middleware, this
/// registry's own copy, a later RPC call) observes the same live value.
pub fn handle_for_thread(ctx: &DispatchCtx, thread_id: &str) -> RunModeHandle {
    registry(ctx).0.handle_for_thread(&(), thread_id)
}

/// Returns the current mode for `thread_id` without creating a handle —
/// `Build` (the default) when the thread has never toggled plan mode.
pub fn get_mode(ctx: &DispatchCtx, thread_id: &str) -> RunMode {
    registry(ctx).0.get_mode(&(), thread_id)
}

/// Sets the mode for `thread_id` (creating a handle if needed) and publishes
/// `DomainEvent::ThreadRunModeChanged` so the web channel can bridge a
/// `run_mode_changed` socket event. A no-op publish-wise when the mode is
/// already what was requested — still safe to call unconditionally.
pub fn set_mode(ctx: &DispatchCtx, thread_id: &str, mode: RunMode) {
    registry(ctx).0.set_mode(&(), thread_id, mode);
}

/// Stable wire label for a [`RunMode`] — `"plan"` / `"build"`.
pub fn mode_label(mode: RunMode) -> &'static str {
    tinyagents_harness_tools::run_mode::mode_label(mode)
}

/// Parses a wire label back into a [`RunMode`]. Unrecognized input maps to
/// `None` so callers can reject it rather than silently defaulting.
pub fn parse_mode_label(label: &str) -> Option<RunMode> {
    tinyagents_harness_tools::run_mode::parse_mode_label(label)
}

// ── JSON-RPC surface ────────────────────────────────────────────────────────
//
// `agent.set_run_mode { thread_id, mode }` / `agent.get_run_mode { thread_id }`
// — lets the composer flip a thread into Plan mode (or read it back) without
// going through a tool call. Colocated here rather than a separate
// `schemas.rs` because this module IS the domain's entire surface (registry
// + RPC), not a multi-file domain directory.

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schema("set_run_mode"), schema("get_run_mode")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema("set_run_mode"),
            handler: handle_set_run_mode,
        },
        RegisteredController {
            schema: schema("get_run_mode"),
            handler: handle_get_run_mode,
        },
    ]
}

fn schema(function: &str) -> ControllerSchema {
    match function {
        "set_run_mode" => ControllerSchema {
            namespace: "agent",
            function: "set_run_mode",
            description: "Set a thread's Plan/Build run mode. Plan mode hides and denies \
                          side-effecting tools (except plan_exit, request_plan_review, todo, \
                          and goal_*) until the thread exits plan mode.",
            inputs: vec![
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment: "The thread to set the mode for.",
                    required: true,
                },
                FieldSchema {
                    name: "mode",
                    ty: TypeSchema::String,
                    comment: "One of `plan` | `build`.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "mode",
                ty: TypeSchema::String,
                comment: "The mode now in effect for the thread.",
                required: true,
            }],
        },
        "get_run_mode" => ControllerSchema {
            namespace: "agent",
            function: "get_run_mode",
            description: "Read a thread's current Plan/Build run mode.",
            inputs: vec![FieldSchema {
                name: "thread_id",
                ty: TypeSchema::String,
                comment: "The thread to read the mode for.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "mode",
                ty: TypeSchema::String,
                comment: "One of `plan` | `build`.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "agent",
            function: "unknown",
            description: "Unknown agent run-mode controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

#[derive(Debug, Deserialize)]
struct ThreadModeParams {
    thread_id: String,
    mode: String,
}

#[derive(Debug, Deserialize)]
struct ThreadIdParams {
    thread_id: String,
}

fn handle_set_run_mode(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ThreadModeParams>(params)?;
        require_owned_thread(&ctx, &p.thread_id).await?;
        let Some(mode) = parse_mode_label(&p.mode) else {
            return Err(format!("invalid mode '{}' (expected plan|build)", p.mode));
        };
        tracing::debug!(
            thread_id = %p.thread_id,
            mode = %p.mode,
            "[rpc][agent] set_run_mode entry"
        );
        set_mode(&ctx, &p.thread_id, mode);
        Ok(serde_json::json!({ "mode": mode_label(mode) }))
    })
}

fn handle_get_run_mode(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = parse::<ThreadIdParams>(params)?;
        require_owned_thread(&ctx, &p.thread_id).await?;
        let mode = get_mode(&ctx, &p.thread_id);
        Ok(serde_json::json!({ "mode": mode_label(mode) }))
    })
}

async fn require_owned_thread(ctx: &DispatchCtx, thread_id: &str) -> Result<(), String> {
    if thread_id.trim().is_empty() {
        return Err("thread_id is required".to_string());
    }
    let config = ctx.load_config().await?;
    let threads =
        crate::threads::store::blocking::list_threads(ctx.clone(), config.workspace_dir).await?;
    if threads.iter().any(|thread| thread.id == thread_id) {
        Ok(())
    } else {
        Err(format!(
            "thread '{thread_id}' not found or not owned by caller"
        ))
    }
}

fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

#[cfg(test)]
#[path = "run_mode_tests.rs"]
mod tests;
