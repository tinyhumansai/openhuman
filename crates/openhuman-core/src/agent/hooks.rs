//! Process-wide hooks supplied by an embedding host.
//!
//! Hook contracts and their per-agent scope are owned by `tinyagents-hooks`;
//! this module only keeps the embedder registries and the host identity bridge.

use std::sync::Arc;

use tinyagents_harness::host::LearningSink;
use tinyagents_hooks::tool::{HookIdentity, ToolHook};

use crate::agent::tinyagents::host::OpenHumanRunContext;

type NamedLearningSink = (Option<String>, Arc<dyn LearningSink>);
type NamedToolHook = (Option<String>, Arc<dyn ToolHook<OpenHumanRunContext>>);

static EMBEDDER_POST_TURN_HOOKS: std::sync::LazyLock<std::sync::Mutex<Vec<NamedLearningSink>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));
static EMBEDDER_TOOL_HOOKS: std::sync::LazyLock<std::sync::Mutex<Vec<NamedToolHook>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

impl HookIdentity for OpenHumanRunContext {
    fn session_id(&self) -> Option<String> {
        self.thread_id.clone()
    }

    fn agent_id(&self) -> Option<String> {
        self.dispatch_ctx()
            .core()
            .and_then(|context| crate::core::runtime::Tenant::of(context).agent)
    }

    fn cwd(&self) -> Option<std::path::PathBuf> {
        self.workspace
            .as_ref()
            .map(|workspace| workspace.root.clone())
            // Without a per-turn descriptor the builtin tools run in the live
            // policy's action dir (see `Shell::effective_action_dir_for_context`),
            // so a hook must report that directory, not the internal workspace.
            .or_else(|| {
                crate::security::live_policy::effective_in(&self.dispatch_ctx())
                    .map(|policy| policy.action_dir.clone())
            })
            .or_else(|| {
                self.dispatch_ctx()
                    .core()
                    .and_then(|context| context.workspace_dir().ok())
            })
    }
}

pub fn register_embedder_post_turn_hook(hook: Arc<dyn LearningSink>) {
    EMBEDDER_POST_TURN_HOOKS
        .lock()
        .expect("embedder learning sinks poisoned")
        .push((None, hook));
}

pub fn replace_embedder_post_turn_hook(name: &str, hook: Option<Arc<dyn LearningSink>>) {
    let mut hooks = EMBEDDER_POST_TURN_HOOKS
        .lock()
        .expect("embedder learning sinks poisoned");
    hooks.retain(|(registered, _)| registered.as_deref() != Some(name));
    if let Some(hook) = hook {
        hooks.push((Some(name.to_owned()), hook));
    }
}

pub fn embedder_post_turn_hooks() -> Vec<Arc<dyn LearningSink>> {
    EMBEDDER_POST_TURN_HOOKS
        .lock()
        .expect("embedder learning sinks poisoned")
        .iter()
        .map(|(_, hook)| Arc::clone(hook))
        .collect()
}

pub fn embedder_post_turn_hook_named(name: &str) -> Option<Arc<dyn LearningSink>> {
    EMBEDDER_POST_TURN_HOOKS
        .lock()
        .expect("embedder learning sinks poisoned")
        .iter()
        .find(|(registered, _)| registered.as_deref() == Some(name))
        .map(|(_, hook)| Arc::clone(hook))
}

pub fn embedder_post_turn_hooks_in(
    ctx: &crate::core::runtime::DispatchCtx,
) -> Vec<Arc<dyn LearningSink>> {
    let mut hooks = embedder_post_turn_hooks();
    if let Some(local) = ctx.host_overrides() {
        hooks.extend(local.post_turn_hooks());
    }
    hooks
}

pub fn register_embedder_tool_hook(hook: Arc<dyn ToolHook<OpenHumanRunContext>>) {
    EMBEDDER_TOOL_HOOKS
        .lock()
        .expect("embedder tool hooks poisoned")
        .push((None, hook));
}

pub fn replace_embedder_tool_hook(
    name: &str,
    hook: Option<Arc<dyn ToolHook<OpenHumanRunContext>>>,
) {
    let mut hooks = EMBEDDER_TOOL_HOOKS
        .lock()
        .expect("embedder tool hooks poisoned");
    hooks.retain(|(registered, _)| registered.as_deref() != Some(name));
    if let Some(hook) = hook {
        hooks.push((Some(name.to_owned()), hook));
    }
}

pub fn embedder_tool_hooks() -> Vec<Arc<dyn ToolHook<OpenHumanRunContext>>> {
    EMBEDDER_TOOL_HOOKS
        .lock()
        .expect("embedder tool hooks poisoned")
        .iter()
        .map(|(_, hook)| Arc::clone(hook))
        .collect()
}

pub fn embedder_tool_hooks_in(
    ctx: &crate::core::runtime::DispatchCtx,
) -> Vec<Arc<dyn ToolHook<OpenHumanRunContext>>> {
    let mut hooks = embedder_tool_hooks();
    if let Some(local) = ctx.host_overrides() {
        hooks.extend(local.tool_hooks());
    }
    hooks
}
