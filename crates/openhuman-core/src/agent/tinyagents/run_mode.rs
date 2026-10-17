//! Per-thread [`RunMode`] registry (Plan vs Build).
//!
//! `tinyagents_harness::middleware::{RunMode, RunModeHandle, plan_mode_middleware}`
//! (vendor tinyagents#211) gate side-effecting tools per-run via a live
//! [`RunModeHandle`] the host can flip without restarting the run. OpenHuman's
//! unit of "a run" for this purpose is a chat *thread*: the same thread is
//! driven through many independent turns (one `install_turn_middleware` call
//! each), so the mode has to live somewhere that outlives any one turn's
//! [`crate::agent::tinyagents::host::OpenHumanRunContext`] — this process-wide,
//! thread_id-keyed registry is that home.
//!
//! `plan_exit` (the tool) and the `agent.set_run_mode` / `agent.get_run_mode`
//! RPCs both read/write through here; `turn_exec` looks the handle up by
//! `OpenHumanRunContext::thread_id` right before assembling each turn's
//! harness and pushes `plan_mode_middleware(handle, ..)` when a handle exists
//! for the thread.

use crate::core::runtime::DispatchCtx;
use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use tinyagents_harness::middleware::{RunMode, RunModeHandle};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;

type RunModeRegistry = Mutex<HashMap<String, RunModeHandle>>;

fn registry(ctx: &DispatchCtx) -> Arc<RunModeRegistry> {
    crate::core::runtime::slot_for::<RunModeRegistry>(ctx)
}

/// Returns the [`RunModeHandle`] for `thread_id`, creating one (starting in
/// [`RunMode::Build`]) if none exists yet. Cloning a `RunModeHandle` shares
/// the same underlying atomic, so every clone (a live turn's middleware, this
/// registry's own copy, a later RPC call) observes the same live value.
pub fn handle_for_thread(ctx: &DispatchCtx, thread_id: &str) -> RunModeHandle {
    let registry = registry(ctx);
    let mut map = registry.lock();
    map.entry(thread_id.to_string())
        .or_insert_with(|| RunModeHandle::new(RunMode::Build))
        .clone()
}

/// Returns the current mode for `thread_id` without creating a handle —
/// `Build` (the default) when the thread has never toggled plan mode.
pub fn get_mode(ctx: &DispatchCtx, thread_id: &str) -> RunMode {
    registry(ctx)
        .lock()
        .get(thread_id)
        .map(|h| h.get())
        .unwrap_or_default()
}

/// Sets the mode for `thread_id` (creating a handle if needed) and publishes
/// `DomainEvent::ThreadRunModeChanged` so the web channel can bridge a
/// `run_mode_changed` socket event. A no-op publish-wise when the mode is
/// already what was requested — still safe to call unconditionally.
pub fn set_mode(ctx: &DispatchCtx, thread_id: &str, mode: RunMode) {
    let handle = handle_for_thread(ctx, thread_id);
    let changed = handle.get() != mode;
    handle.set(mode);
    if changed {
        tracing::info!(
            thread_id = %thread_id,
            mode = mode_label(mode),
            "[agent::run_mode] thread run mode changed"
        );
        BUS.publish(DomainEvent::ThreadRunModeChanged {
            thread_id: thread_id.to_string(),
            mode: mode_label(mode).to_string(),
        });
    }
}

/// Stable wire label for a [`RunMode`] — `"plan"` / `"build"`.
pub fn mode_label(mode: RunMode) -> &'static str {
    match mode {
        RunMode::Plan => "plan",
        RunMode::Build => "build",
    }
}

/// Parses a wire label back into a [`RunMode`]. Unrecognized input maps to
/// `None` so callers can reject it rather than silently defaulting.
pub fn parse_mode_label(label: &str) -> Option<RunMode> {
    match label {
        "plan" => Some(RunMode::Plan),
        "build" => Some(RunMode::Build),
        _ => None,
    }
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
        let mode = get_mode(&ctx, &p.thread_id);
        Ok(serde_json::json!({ "mode": mode_label(mode) }))
    })
}

fn parse<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

#[cfg(test)]
#[path = "run_mode_tests.rs"]
mod tests;
