//! Wire types for the configurable hook system.
//!
//! The vocabulary is deliberately close to Cursor's `hooks.json` contract
//! (<https://cursor.com/docs/hooks>) so a hook script written for one host runs
//! unchanged on the other: same event names, same stdin envelope, same stdout
//! decision object, same exit-code semantics.
//!
//! Two things are *not* copied verbatim, and both are widenings rather than
//! divergences:
//!
//! * **Event names are matched loosely.** [`HookEvent::parse`] normalizes by
//!   lowercasing and dropping separators, so `preToolUse`, `PreToolUse` and
//!   `pre_tool_use` are the same event. A `hooks.json` authored for Cursor,
//!   for Claude Code, or by hand all resolve.
//! * **One output struct serves every event.** A hook may return any subset of
//!   fields; the engine honours only the ones its event defines. That keeps a
//!   generic "deny everything from this script" hook usable across events
//!   without the author tracking which schema each one wants.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A lifecycle moment a configured hook can observe or gate.
///
/// Ordering of the variants follows the lifecycle, not the alphabet: session →
/// prompt → tool → shell/file specialisations → subagent → wrap-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum HookEvent {
    /// A new agent session was created.
    #[serde(rename = "sessionStart")]
    SessionStart,
    /// A session finished, was aborted, or errored.
    #[serde(rename = "sessionEnd")]
    SessionEnd,
    /// The user submitted a prompt, before it reaches the model.
    #[serde(rename = "beforeSubmitPrompt")]
    BeforeSubmitPrompt,
    /// Immediately before a tool executes. Can deny or rewrite the arguments.
    #[serde(rename = "preToolUse")]
    PreToolUse,
    /// After a tool returned successfully. Can append context for the model.
    #[serde(rename = "postToolUse")]
    PostToolUse,
    /// After a tool failed, timed out, or was denied.
    #[serde(rename = "postToolUseFailure")]
    PostToolUseFailure,
    /// Before a shell command runs — the shell-tool specialisation of
    /// [`Self::PreToolUse`], carrying the command line rather than raw args.
    #[serde(rename = "beforeShellExecution")]
    BeforeShellExecution,
    /// After a shell command completed.
    #[serde(rename = "afterShellExecution")]
    AfterShellExecution,
    /// Before an MCP tool call leaves the process.
    #[serde(rename = "beforeMCPExecution")]
    BeforeMcpExecution,
    /// After an MCP tool call returned.
    #[serde(rename = "afterMCPExecution")]
    AfterMcpExecution,
    /// Before a file is read into the model's context.
    #[serde(rename = "beforeReadFile")]
    BeforeReadFile,
    /// After a file was written or edited on disk.
    #[serde(rename = "afterFileEdit")]
    AfterFileEdit,
    /// A subagent is about to start.
    #[serde(rename = "subagentStart")]
    SubagentStart,
    /// A subagent finished.
    #[serde(rename = "subagentStop")]
    SubagentStop,
    /// The transcript is about to be compacted.
    #[serde(rename = "preCompact")]
    PreCompact,
    /// The assistant produced a message.
    #[serde(rename = "afterAgentResponse")]
    AfterAgentResponse,
    /// The assistant produced a reasoning block.
    #[serde(rename = "afterAgentThought")]
    AfterAgentThought,
    /// The agent loop finished a turn. Can inject a follow-up message.
    #[serde(rename = "stop")]
    Stop,
}

impl HookEvent {
    /// Every event, in lifecycle order. Used by the RPC surface and by the
    /// config validator to report unknown event keys with a usable suggestion.
    pub const ALL: [HookEvent; 18] = [
        HookEvent::SessionStart,
        HookEvent::SessionEnd,
        HookEvent::BeforeSubmitPrompt,
        HookEvent::PreToolUse,
        HookEvent::PostToolUse,
        HookEvent::PostToolUseFailure,
        HookEvent::BeforeShellExecution,
        HookEvent::AfterShellExecution,
        HookEvent::BeforeMcpExecution,
        HookEvent::AfterMcpExecution,
        HookEvent::BeforeReadFile,
        HookEvent::AfterFileEdit,
        HookEvent::SubagentStart,
        HookEvent::SubagentStop,
        HookEvent::PreCompact,
        HookEvent::AfterAgentResponse,
        HookEvent::AfterAgentThought,
        HookEvent::Stop,
    ];

    /// The canonical wire name, as it appears in `hooks.json` and in the
    /// `hook_event_name` field of the stdin envelope.
    pub fn as_str(self) -> &'static str {
        match self {
            HookEvent::SessionStart => "sessionStart",
            HookEvent::SessionEnd => "sessionEnd",
            HookEvent::BeforeSubmitPrompt => "beforeSubmitPrompt",
            HookEvent::PreToolUse => "preToolUse",
            HookEvent::PostToolUse => "postToolUse",
            HookEvent::PostToolUseFailure => "postToolUseFailure",
            HookEvent::BeforeShellExecution => "beforeShellExecution",
            HookEvent::AfterShellExecution => "afterShellExecution",
            HookEvent::BeforeMcpExecution => "beforeMCPExecution",
            HookEvent::AfterMcpExecution => "afterMCPExecution",
            HookEvent::BeforeReadFile => "beforeReadFile",
            HookEvent::AfterFileEdit => "afterFileEdit",
            HookEvent::SubagentStart => "subagentStart",
            HookEvent::SubagentStop => "subagentStop",
            HookEvent::PreCompact => "preCompact",
            HookEvent::AfterAgentResponse => "afterAgentResponse",
            HookEvent::AfterAgentThought => "afterAgentThought",
            HookEvent::Stop => "stop",
        }
    }

    /// Resolve a config key to an event, tolerating casing and separators.
    ///
    /// `preToolUse`, `PreToolUse`, `pre_tool_use` and `pre-tool-use` are the
    /// same event; so are Claude Code's `UserPromptSubmit` /
    /// `SubagentStop` spellings, which alias onto the closest OpenHuman moment.
    pub fn parse(key: &str) -> Option<HookEvent> {
        let normalized = normalize_key(key);
        if let Some(event) = HookEvent::ALL
            .iter()
            .copied()
            .find(|event| normalize_key(event.as_str()) == normalized)
        {
            return Some(event);
        }
        // Aliases from neighbouring hook dialects.
        match normalized.as_str() {
            "userpromptsubmit" => Some(HookEvent::BeforeSubmitPrompt),
            "notification" | "agentresponse" => Some(HookEvent::AfterAgentResponse),
            "agentthought" => Some(HookEvent::AfterAgentThought),
            "precompaction" | "compact" => Some(HookEvent::PreCompact),
            "sessionstop" => Some(HookEvent::SessionEnd),
            "toolusefailure" | "tooluseerror" => Some(HookEvent::PostToolUseFailure),
            _ => None,
        }
    }

    /// Whether the core currently fires this event.
    ///
    /// Every event in [`Self::ALL`] is a complete contract — it parses, it
    /// matches, it executes, and `hooks.test` fires it — but a few have no call
    /// site in the harness yet. Those return `false`, and the loader warns when
    /// a `hooks.json` registers one, because a hook that silently never runs is
    /// the worst outcome this system can produce: the author believes a policy
    /// is enforced and nothing says otherwise.
    ///
    /// Move an event here as its call site lands; do not flip one optimistically.
    pub fn is_wired(self) -> bool {
        !matches!(
            self,
            // No async seam at session construction yet — `AgentBuilder::build`
            // is synchronous, and `sessionStart` has to be able to return
            // context that reaches the system prompt.
            HookEvent::SessionStart
                | HookEvent::SessionEnd
                // Compaction runs inside the tinyagents middleware, which owns
                // its own trigger; surfacing it needs a seam upstream.
                | HookEvent::PreCompact
                // Reasoning blocks are not projected onto the post-turn seam
                // the bridge rides.
                | HookEvent::AfterAgentThought
                // `ops::subagent_stopped` exists and is complete, but nothing
                // calls it: `subagent_runner` fires the start side and returns
                // from a dozen places without a stop. Until one lands, a
                // configured `subagentStop` hook never runs.
                | HookEvent::SubagentStop
        )
    }

    /// Whether a decision from this event can change what the core does.
    ///
    /// Gating events are run **sequentially** and their output is honoured;
    /// observational events are fire-and-forget, so the engine never blocks a
    /// turn on one. This is the single place that distinction is encoded.
    pub fn is_gating(self) -> bool {
        matches!(
            self,
            HookEvent::PreToolUse
                | HookEvent::BeforeShellExecution
                | HookEvent::BeforeMcpExecution
                | HookEvent::BeforeReadFile
                | HookEvent::BeforeSubmitPrompt
                | HookEvent::SubagentStart
                | HookEvent::SessionStart
                | HookEvent::PostToolUse
                | HookEvent::Stop
                | HookEvent::SubagentStop
                | HookEvent::PreCompact
        )
    }
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Lowercase and drop every separator, so hook-name dialects converge.
pub(crate) fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The permission verdict a gating hook may return.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum HookPermission {
    /// Proceed. The absence of a verdict means the same thing.
    #[default]
    Allow,
    /// Refuse the action. The agent is told why.
    Deny,
    /// Escalate to the human through the existing approval gate.
    Ask,
}

/// The envelope handed to every hook on stdin.
///
/// Fields common to all events sit at the top level; the event-specific body is
/// flattened in from [`HookPayload`], matching Cursor's shape where one JSON
/// object carries both.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookInput {
    /// The event that fired, as its canonical wire name.
    pub hook_event_name: String,
    /// Conversation/thread identifier, when the moment belongs to one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    /// Per-turn generation identifier, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_id: Option<String>,
    /// Agent session identifier, when the moment belongs to a session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Model driving the turn, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Canonical agent definition id, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Core version string, so a hook can gate on host capability.
    pub openhuman_version: String,
    /// Filesystem roots the agent may act in — `action_dir` plus any turn
    /// workspace. First entry is the primary root.
    pub workspace_roots: Vec<String>,
    /// Working directory the action is scoped to, when the event has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// The event body.
    #[serde(flatten)]
    pub payload: HookPayload,
}

/// Event-specific fields, flattened into [`HookInput`].
///
/// Untagged on purpose: the discriminator a hook script reads is
/// `hook_event_name`, exactly as in Cursor, so the body must not add a second
/// tag key of its own.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(untagged)]
pub enum HookPayload {
    /// Events that carry no body beyond the common envelope.
    #[default]
    Empty,
    /// Tool lifecycle: pre, post, and failure.
    Tool(ToolPayload),
    /// Shell command execution.
    Shell(ShellPayload),
    /// File read or edit.
    File(FilePayload),
    /// Prompt submission.
    Prompt(PromptPayload),
    /// Subagent start/stop.
    Subagent(SubagentPayload),
    /// Session start/end.
    Session(SessionPayload),
    /// Transcript compaction.
    Compact(CompactPayload),
    /// Assistant text or reasoning.
    Text(TextPayload),
    /// Turn completion.
    Stop(StopPayload),
}

impl HookPayload {
    /// Parse a body **for a known event**.
    ///
    /// Never rely on `serde`'s untagged dispatch for this. Several bodies have
    /// only optional fields, so untagged matching picks whichever variant is
    /// declared first and happens to accept the object — `{"trigger":"auto"}`
    /// deserializes as a [`SessionPayload`] with everything `None`, silently
    /// throwing away the field the caller sent. The event already says which
    /// body this is; use it.
    pub fn from_value_for(event: HookEvent, value: serde_json::Value) -> Result<Self, String> {
        fn parse<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> Result<T, String> {
            serde_json::from_value(value).map_err(|error| error.to_string())
        }
        if value.is_null() {
            return Ok(HookPayload::Empty);
        }
        Ok(match event {
            HookEvent::PreToolUse
            | HookEvent::PostToolUse
            | HookEvent::PostToolUseFailure
            | HookEvent::BeforeMcpExecution
            | HookEvent::AfterMcpExecution => HookPayload::Tool(parse(value)?),
            HookEvent::BeforeShellExecution | HookEvent::AfterShellExecution => {
                HookPayload::Shell(parse(value)?)
            }
            HookEvent::BeforeReadFile | HookEvent::AfterFileEdit => {
                HookPayload::File(parse(value)?)
            }
            HookEvent::BeforeSubmitPrompt => HookPayload::Prompt(parse(value)?),
            HookEvent::SubagentStart | HookEvent::SubagentStop => {
                HookPayload::Subagent(parse(value)?)
            }
            HookEvent::SessionStart | HookEvent::SessionEnd => HookPayload::Session(parse(value)?),
            HookEvent::PreCompact => HookPayload::Compact(parse(value)?),
            HookEvent::AfterAgentResponse | HookEvent::AfterAgentThought => {
                HookPayload::Text(parse(value)?)
            }
            HookEvent::Stop => HookPayload::Stop(parse(value)?),
        })
    }
}

/// Body for `preToolUse`, `postToolUse`, `postToolUseFailure`, and the MCP
/// specialisations.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolPayload {
    /// Registered tool name.
    pub tool_name: String,
    /// Arguments after argument-recovery normalization.
    pub tool_input: serde_json::Value,
    /// Provider call id, correlating the pre and post events.
    pub tool_use_id: String,
    /// Tool output, JSON-stringified. Absent before execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_output: Option<String>,
    /// Wall-clock tool runtime in milliseconds. Absent before execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Failure text, on `postToolUseFailure`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Failure class: `timeout`, `error`, or `permission_denied`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_type: Option<String>,
}

/// Body for `beforeShellExecution` / `afterShellExecution`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShellPayload {
    /// The full command line as it will be handed to the shell.
    pub command: String,
    /// Whether the command runs inside a sandbox backend.
    pub sandbox: bool,
    /// Combined output. Absent before execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Wall-clock runtime in milliseconds. Absent before execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Body for `beforeReadFile` / `afterFileEdit`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FilePayload {
    /// Absolute path of the file.
    pub file_path: String,
    /// Applied edits, on `afterFileEdit`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<FileEdit>,
}

/// A single string replacement within a file.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileEdit {
    /// Text that was replaced. Empty for a whole-file write.
    pub old_string: String,
    /// Text that replaced it.
    pub new_string: String,
}

/// Body for `beforeSubmitPrompt`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PromptPayload {
    /// The user's message.
    pub prompt: String,
    /// Attachment paths riding along with the prompt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<String>,
}

/// Body for `subagentStart` / `subagentStop`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubagentPayload {
    /// Agent definition id of the child.
    pub subagent_type: String,
    /// The task handed to the child.
    pub task: String,
    /// Parent conversation identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_conversation_id: Option<String>,
    /// Terminal status on stop: `completed`, `error`, or `aborted`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Child runtime in milliseconds, on stop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Body for `sessionStart` / `sessionEnd`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionPayload {
    /// Entrypoint that created the session (`cli`, `web_chat`, `cron`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    /// Why the session ended, on `sessionEnd`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Session lifetime in milliseconds, on `sessionEnd`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Body for `preCompact`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompactPayload {
    /// `auto` or `manual`.
    pub trigger: String,
    /// Context occupancy as a percentage of the window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_usage_percent: Option<f64>,
    /// Messages in the transcript at the moment of compaction.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_count: Option<usize>,
}

/// Body for `afterAgentResponse` / `afterAgentThought`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TextPayload {
    /// The assistant text or reasoning block.
    pub text: String,
    /// Reasoning duration in milliseconds, for thoughts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// Body for `stop`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StopPayload {
    /// `completed`, `aborted`, or `error`.
    pub status: String,
    /// How many times this turn has already been extended by a follow-up.
    pub loop_count: u32,
    /// Model calls made during the turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iteration_count: Option<usize>,
}

/// What a hook may return on stdout.
///
/// Every field is optional and every event honours only the subset it defines,
/// so one script can serve several events without branching on the schema.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookOutput {
    /// Verdict for a gating event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<HookPermission>,
    /// Shown to the human. Never enters the model transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_message: Option<String>,
    /// Shown to the model — the reason a denial happened, or a nudge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_message: Option<String>,
    /// Replacement tool arguments, honoured on `preToolUse`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<serde_json::Value>,
    /// Text appended to a tool result or to the session's system context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
    /// `false` aborts prompt submission on `beforeSubmitPrompt`.
    #[serde(default, rename = "continue", skip_serializing_if = "Option::is_none")]
    pub continue_: Option<bool>,
    /// Injects another user turn on `stop` / `subagentStop`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub followup_message: Option<String>,
    /// Environment variables added to every later hook in the session,
    /// honoured on `sessionStart`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,
}

impl HookOutput {
    /// A denial carrying the reason the model should see.
    pub fn deny(agent_message: impl Into<String>) -> Self {
        Self {
            permission: Some(HookPermission::Deny),
            agent_message: Some(agent_message.into()),
            ..Self::default()
        }
    }

    /// Whether this output refuses the action.
    pub fn is_deny(&self) -> bool {
        self.permission == Some(HookPermission::Deny)
    }

    /// Whether this output escalates to the human.
    pub fn is_ask(&self) -> bool {
        self.permission == Some(HookPermission::Ask)
    }

    /// Fold a later hook's output into this one.
    ///
    /// Denial is sticky and beats `ask`, which beats `allow` — the strictest
    /// verdict from any hook wins, so adding a hook can never loosen a policy
    /// another one set. Text fields concatenate; `updated_input` and `env`
    /// are last-writer-wins because merging two rewrites of the same arguments
    /// has no defensible semantics.
    pub fn merge(&mut self, other: HookOutput) {
        self.permission = match (self.permission, other.permission) {
            (Some(HookPermission::Deny), _) | (_, Some(HookPermission::Deny)) => {
                Some(HookPermission::Deny)
            }
            (Some(HookPermission::Ask), _) | (_, Some(HookPermission::Ask)) => {
                Some(HookPermission::Ask)
            }
            (Some(HookPermission::Allow), _) | (_, Some(HookPermission::Allow)) => {
                Some(HookPermission::Allow)
            }
            (None, None) => None,
        };
        append_message(&mut self.user_message, other.user_message);
        append_message(&mut self.agent_message, other.agent_message);
        append_message(&mut self.additional_context, other.additional_context);
        append_message(&mut self.followup_message, other.followup_message);
        if other.updated_input.is_some() {
            self.updated_input = other.updated_input;
        }
        if let Some(env) = other.env {
            self.env.get_or_insert_with(BTreeMap::new).extend(env);
        }
        // `continue: false` is sticky for the same reason denial is.
        if other.continue_ == Some(false) || self.continue_ == Some(false) {
            self.continue_ = Some(false);
        } else if other.continue_.is_some() {
            self.continue_ = other.continue_;
        }
    }
}

fn append_message(slot: &mut Option<String>, addition: Option<String>) {
    let Some(addition) = addition.filter(|text| !text.is_empty()) else {
        return;
    };
    match slot {
        Some(existing) => {
            existing.push('\n');
            existing.push_str(&addition);
        }
        None => *slot = Some(addition),
    }
}
