//! Wiring the configured hooks into the agent harness.
//!
//! The harness already carries in-process hook seams —
//! [`ToolHook`](crate::openhuman::agent::hooks::ToolHook) around every tool and
//! [`PostTurnHook`](crate::openhuman::agent::hooks::PostTurnHook) after every
//! turn. Rather than adding a second set of call sites, the configurable engine
//! registers itself *through* those seams. One bridge object, registered once
//! at bootstrap, turns every configured `hooks.json` entry into behaviour.
//!
//! ## Derived events
//!
//! Cursor exposes `beforeShellExecution`, `beforeReadFile` and `afterFileEdit`
//! as first-class events. In OpenHuman those moments are not separate call
//! sites — they are the `shell`, `file_read` and `file_write`/`edit` tools
//! going through the ordinary tool seam. So the bridge *derives* them: when a
//! tool call matches one of those families it fires both the generic
//! `preToolUse` event and the specialised one, with a payload shaped the way a
//! Cursor hook expects (a command line, a file path) rather than raw tool
//! arguments.
//!
//! This is what makes a Cursor hook script portable here. The alternative —
//! only exposing `preToolUse` and telling authors to parse `tool_input` — would
//! have been less code and a worse contract: every author would reimplement the
//! same mapping, and each would get the sandbox flag and the path resolution
//! subtly differently.

use async_trait::async_trait;
use serde_json::Value;

use crate::openhuman::agent::hooks::{
    PostTurnHook, ToolHook, ToolHookContext, ToolHookDecision, TurnContext,
};

use super::context::{build_input, TurnIdentity};
use super::engine::{self, HookOutcome};
use super::types::{
    FileEdit, FilePayload, HookEvent, HookOutput, HookPayload, ShellPayload, StopPayload,
    TextPayload, ToolPayload,
};

/// Name the bridge registers itself under, so a host that rebuilds its core can
/// replace rather than duplicate it.
pub const BRIDGE_HOOK_NAME: &str = "configured_hooks";

/// Tools whose calls are also reported as shell execution.
const SHELL_TOOLS: &[&str] = &["shell", "run_command", "bash", "node_exec", "npm_exec"];

/// Tools whose calls are also reported as a file read.
const READ_TOOLS: &[&str] = &["file_read", "read_diff"];

/// Tools whose calls are also reported as a file edit.
const WRITE_TOOLS: &[&str] = &["file_write", "edit", "apply_patch", "update_memory_md"];

/// Bridges the configured hook engine onto the harness hook seams.
#[derive(Debug, Default, Clone, Copy)]
pub struct ConfiguredHookBridge;

impl ConfiguredHookBridge {
    /// Register the bridge for every subsequently-created session.
    ///
    /// Replaces any previously registered bridge rather than appending, so a
    /// host that rebuilds its core does not end up firing every hook twice.
    pub fn install() {
        crate::openhuman::agent::hooks::replace_embedder_tool_hook(
            BRIDGE_HOOK_NAME,
            Some(std::sync::Arc::new(ConfiguredHookBridge)),
        );
        crate::openhuman::agent::hooks::replace_embedder_post_turn_hook(
            BRIDGE_HOOK_NAME,
            Some(std::sync::Arc::new(ConfiguredHookBridge)),
        );
        log::debug!("[hooks] configured-hook bridge installed");
    }

    /// Remove the bridge. Used when hooks are disabled by config.
    pub fn uninstall() {
        crate::openhuman::agent::hooks::replace_embedder_tool_hook(BRIDGE_HOOK_NAME, None);
        crate::openhuman::agent::hooks::replace_embedder_post_turn_hook(BRIDGE_HOOK_NAME, None);
        log::debug!("[hooks] configured-hook bridge removed");
    }
}

fn identity_from_tool(context: &ToolHookContext) -> TurnIdentity {
    TurnIdentity {
        session_id: context.session_id.clone(),
        agent_id: context.agent_id.clone(),
        cwd: string_field(&context.arguments, "cwd"),
        ..TurnIdentity::default()
    }
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|text| !text.is_empty())
}

fn tool_payload(context: &ToolHookContext) -> ToolPayload {
    ToolPayload {
        tool_name: context.tool_name.clone(),
        tool_input: context.arguments.clone(),
        tool_use_id: context.call_id.clone(),
        tool_output: context.output.clone(),
        duration_ms: context.duration_ms,
        error_message: context.error.clone(),
        failure_type: context.error.as_deref().map(classify_failure),
    }
}

/// Map a raw failure string onto Cursor's three failure classes.
fn classify_failure(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("timed out") || lower.contains("timeout") {
        "timeout".to_string()
    } else if lower.contains("policy-blocked")
        || lower.contains("permission")
        || lower.contains("denied")
    {
        "permission_denied".to_string()
    } else {
        "error".to_string()
    }
}

/// Which specialised event, if any, this tool also represents.
fn derived_event(tool_name: &str, pre: bool) -> Option<HookEvent> {
    let name = tool_name.to_ascii_lowercase();
    if SHELL_TOOLS.contains(&name.as_str()) {
        return Some(if pre {
            HookEvent::BeforeShellExecution
        } else {
            HookEvent::AfterShellExecution
        });
    }
    if READ_TOOLS.contains(&name.as_str()) {
        // There is no `afterReadFile`; a read only has a gating moment.
        return pre.then_some(HookEvent::BeforeReadFile);
    }
    if WRITE_TOOLS.contains(&name.as_str()) {
        // A write is only reported once it happened — denying it belongs to
        // `preToolUse`, which already fired with the same arguments.
        return (!pre).then_some(HookEvent::AfterFileEdit);
    }
    if is_mcp_tool(&name) {
        return Some(if pre {
            HookEvent::BeforeMcpExecution
        } else {
            HookEvent::AfterMcpExecution
        });
    }
    None
}

/// MCP tools reach the registry through the `mcp_*` families; a dynamically
/// installed server's tools are namespaced with `mcp:`.
fn is_mcp_tool(name: &str) -> bool {
    name.starts_with("mcp_") || name.starts_with("mcp:") || name.starts_with("mcp__")
}

/// Build the payload the specialised event expects from raw tool arguments.
fn derived_payload(event: HookEvent, context: &ToolHookContext) -> HookPayload {
    match event {
        HookEvent::BeforeShellExecution | HookEvent::AfterShellExecution => {
            HookPayload::Shell(ShellPayload {
                command: string_field(&context.arguments, "command").unwrap_or_default(),
                sandbox: context
                    .arguments
                    .get("sandbox")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                output: context.output.clone(),
                duration_ms: context.duration_ms,
            })
        }
        HookEvent::BeforeReadFile => HookPayload::File(FilePayload {
            file_path: file_path_argument(&context.arguments),
            edits: Vec::new(),
        }),
        HookEvent::AfterFileEdit => HookPayload::File(FilePayload {
            file_path: file_path_argument(&context.arguments),
            edits: file_edits(&context.arguments),
        }),
        _ => HookPayload::Tool(tool_payload(context)),
    }
}

/// The file tools spell their path argument three different ways; a hook author
/// should not have to know which tool used which.
fn file_path_argument(arguments: &Value) -> String {
    ["path", "file_path", "filename", "file"]
        .iter()
        .find_map(|key| string_field(arguments, key))
        .unwrap_or_default()
}

/// Recover the before/after strings an edit applied, where the tool exposes
/// them. A whole-file write reports one edit whose `old_string` is empty.
fn file_edits(arguments: &Value) -> Vec<FileEdit> {
    if let (Some(old), Some(new)) = (
        string_field(arguments, "old_string"),
        string_field(arguments, "new_string"),
    ) {
        return vec![FileEdit {
            old_string: old,
            new_string: new,
        }];
    }
    match string_field(arguments, "content") {
        Some(content) => vec![FileEdit {
            old_string: String::new(),
            new_string: content,
        }],
        None => Vec::new(),
    }
}

/// Fire the generic event and, when the tool has one, its specialisation.
async fn dispatch_pair(
    generic: HookEvent,
    context: &ToolHookContext,
    identity: TurnIdentity,
) -> HookOutcome {
    let engine = engine::global();
    let mut merged = HookOutcome::default();

    if engine.has_hooks(generic).await {
        let input = build_input(
            generic,
            identity.clone(),
            HookPayload::Tool(tool_payload(context)),
        );
        let outcome = engine.dispatch(generic, input).await;
        merged.output.merge(outcome.output);
        merged.runs.extend(outcome.runs);
    }
    if merged.output.is_deny() {
        return merged;
    }

    let pre = matches!(
        generic,
        HookEvent::PreToolUse | HookEvent::BeforeMcpExecution | HookEvent::BeforeReadFile
    );
    if let Some(event) = derived_event(&context.tool_name, pre) {
        if engine.has_hooks(event).await {
            let input = build_input(event, identity, derived_payload(event, context));
            let outcome = engine.dispatch(event, input).await;
            merged.output.merge(outcome.output);
            merged.runs.extend(outcome.runs);
        }
    }
    merged
}

#[async_trait]
impl ToolHook for ConfiguredHookBridge {
    fn name(&self) -> &str {
        BRIDGE_HOOK_NAME
    }

    async fn before_tool(&self, context: &ToolHookContext) -> anyhow::Result<()> {
        match self.before_tool_decision(context).await {
            ToolHookDecision::Deny(reason) => Err(anyhow::anyhow!(reason)),
            _ => Ok(()),
        }
    }

    async fn after_tool(&self, context: &ToolHookContext) -> anyhow::Result<()> {
        self.after_tool_context(context).await;
        Ok(())
    }

    async fn before_tool_decision(&self, context: &ToolHookContext) -> ToolHookDecision {
        let identity = identity_from_tool(context);
        let outcome = dispatch_pair(HookEvent::PreToolUse, context, identity).await;
        decision_from(outcome.output, &context.tool_name)
    }

    async fn after_tool_context(&self, context: &ToolHookContext) -> Option<String> {
        let identity = identity_from_tool(context);
        let generic = if context.success == Some(false) {
            HookEvent::PostToolUseFailure
        } else {
            HookEvent::PostToolUse
        };
        let outcome = dispatch_pair(generic, context, identity).await;
        outcome.output.additional_context
    }
}

/// Translate a merged hook decision into the harness's tool verdict.
fn decision_from(output: HookOutput, tool_name: &str) -> ToolHookDecision {
    if output.is_deny() {
        let reason = output
            .agent_message
            .or(output.user_message)
            .unwrap_or_else(|| format!("a configured hook denied {tool_name}"));
        return ToolHookDecision::Deny(reason);
    }
    if output.is_ask() {
        let reason = output
            .user_message
            .or(output.agent_message)
            .unwrap_or_else(|| format!("a configured hook wants approval for {tool_name}"));
        return ToolHookDecision::Ask(reason);
    }
    match output.updated_input {
        Some(arguments) => ToolHookDecision::ProceedWith(arguments),
        None => ToolHookDecision::Proceed,
    }
}

#[async_trait]
impl PostTurnHook for ConfiguredHookBridge {
    fn name(&self) -> &str {
        BRIDGE_HOOK_NAME
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        let engine = engine::global();
        let identity = TurnIdentity {
            session_id: ctx.session_id.clone(),
            agent_id: ctx.agent_id.clone(),
            conversation_id: ctx.session_id.clone(),
            ..TurnIdentity::default()
        };

        if engine.has_hooks(HookEvent::AfterAgentResponse).await {
            let input = build_input(
                HookEvent::AfterAgentResponse,
                identity.clone(),
                HookPayload::Text(TextPayload {
                    text: ctx.assistant_response.clone(),
                    duration_ms: Some(ctx.turn_duration_ms),
                }),
            );
            engine.dispatch(HookEvent::AfterAgentResponse, input).await;
        }

        if engine.has_hooks(HookEvent::Stop).await {
            let input = build_input(
                HookEvent::Stop,
                identity,
                HookPayload::Stop(StopPayload {
                    status: "completed".to_string(),
                    loop_count: 0,
                    iteration_count: Some(ctx.iteration_count),
                }),
            );
            let outcome = engine.dispatch(HookEvent::Stop, input).await;
            if let Some(followup) = outcome.output.followup_message {
                // The turn has already returned by the time a post-turn hook
                // runs, so a follow-up cannot re-enter this turn. Publishing it
                // lets the entrypoint that owns the conversation decide whether
                // to start another one — the only layer that knows if there is
                // still a user attached.
                super::followup::publish(ctx.session_id.clone(), followup).await;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "bridge_tests.rs"]
mod tests;
