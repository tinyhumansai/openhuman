//! The channel runtime's system prompt: fixed for tests, identity-refreshing
//! in production.
//!
//! Two defects lived in the old `Arc<String>`:
//!
//! - **#6027** — the native channel runtime rendered its prompt from the
//!   workspace-root identity files and did not refresh their contents, so
//!   channel replies could use stale identity context after an edit.
//! - **#6028** — the prompt was rendered once in `start_channels` and kept
//!   for the life of the process, so a `SOUL.md` edit stayed invisible until
//!   restart.
//!
//! [`ChannelSystemPrompt::refreshing`] keeps the *inputs* of the render (tool
//! descriptions, skills, model, and the tool-instruction + access-context
//! suffix, all fixed for the process) and re-renders only when an identity
//! fingerprint changes. The fingerprint is a hash of `(mtime, len)` for the
//! files the identity is read from, so the steady state costs a few `stat`s
//! per message and hands back the same `Arc<String>` — the prompt stays
//! byte-stable within a run for prefix-cache hits, the property
//! `channels_prompt.rs` documents. Only an identity change produces new
//! bytes, which is the whole point.

use crate::agent::context::channels_prompt::{
    build_system_prompt_with_identity, render_project_context, ProjectContextPlacement,
    PromptIdentityOverride,
};
use crate::skills::Workflow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

/// Workspace-root files `build_system_prompt` inlines.
const ROOT_IDENTITY_FILES: [&str; 2] = ["SOUL.md", "IDENTITY.md"];

/// Everything the channel prompt is rendered from except the identity files.
/// Fixed for the life of the process, as before: tools, skills, the model
/// and the security posture still need a restart to change.
#[derive(Debug, Clone)]
pub(crate) struct ChannelPromptInputs {
    pub(crate) workspace_dir: PathBuf,
    pub(crate) model: String,
    pub(crate) tool_descs: Vec<(String, String)>,
    pub(crate) skills: Vec<Workflow>,
    pub(crate) bootstrap_max_chars: Option<usize>,
    /// Appended verbatim after the rendered prompt: the tool-instruction
    /// block and the access-context block, both rendered once at boot.
    pub(crate) suffix: String,
}

/// The identity one render was produced from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelIdentity {}

impl ChannelIdentity {
    /// The identity represented by the workspace-root files.
    fn root() -> Self {
        Self {}
    }
}

struct PromptState {
    fingerprint: u64,
    rendered: Arc<String>,
}

/// Shared state behind [`ChannelSystemPrompt::Refreshing`].
pub(crate) struct RefreshingInner {
    inputs: ChannelPromptInputs,
    state: Mutex<PromptState>,
}

/// The system prompt a channel turn is seeded with.
///
/// Clone-shared: every clone of a `Refreshing` prompt observes the same
/// cache, so the test harness can hand one prompt to several dispatches.
#[derive(Clone)]
pub(crate) enum ChannelSystemPrompt {
    /// A literal prompt that never changes — tests and bespoke hosts.
    Fixed(Arc<String>),
    /// Rendered from [`ChannelPromptInputs`] and re-rendered when the
    /// identity files or the active profile change.
    Refreshing(Arc<RefreshingInner>),
}

impl std::fmt::Debug for ChannelSystemPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fixed(text) => f
                .debug_struct("ChannelSystemPrompt::Fixed")
                .field("chars", &text.chars().count())
                .finish(),
            Self::Refreshing(inner) => {
                let state = inner.state.lock().unwrap_or_else(|e| e.into_inner());
                f.debug_struct("ChannelSystemPrompt::Refreshing")
                    .field("workspace_dir", &inner.inputs.workspace_dir)
                    .field("chars", &state.rendered.chars().count())
                    .finish()
            }
        }
    }
}

impl ChannelSystemPrompt {
    /// A prompt that is exactly `text`, forever.
    pub(crate) fn fixed(text: impl Into<String>) -> Self {
        Self::Fixed(Arc::new(text.into()))
    }

    /// Renders the prompt for the current identity now — `start_channels`
    /// needs a prompt before the first message — and keeps `inputs` to
    /// re-render when the identity changes.
    pub(crate) fn refreshing(inputs: ChannelPromptInputs) -> Self {
        let identity = ChannelIdentity::root();
        let fingerprint = identity_fingerprint(&inputs.workspace_dir);
        let rendered = Arc::new(render(&inputs, &identity));
        log_render(&inputs.workspace_dir, &identity, &rendered, "boot");
        Self::Refreshing(Arc::new(RefreshingInner {
            inputs,
            state: Mutex::new(PromptState {
                fingerprint,
                rendered,
            }),
        }))
    }

    /// The prompt to seed the next turn with. For a refreshing prompt this
    /// checks the identity fingerprint and re-renders on a mismatch; when
    /// nothing changed it returns the cached `Arc`, pointer-equal to the
    /// previous call's.
    pub(crate) fn current(&self) -> Arc<String> {
        match self {
            Self::Fixed(text) => Arc::clone(text),
            Self::Refreshing(inner) => inner.current(),
        }
    }
}

impl RefreshingInner {
    fn current(&self) -> Arc<String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let workspace_dir = &self.inputs.workspace_dir;
        let observed = identity_fingerprint(workspace_dir);
        if observed == state.fingerprint {
            return Arc::clone(&state.rendered);
        }

        tracing::debug!(
            target: "openhuman::channels",
            "[channels][prompt] identity fingerprint changed; re-rendering system prompt"
        );
        let identity = ChannelIdentity::root();
        let fingerprint = identity_fingerprint(workspace_dir);
        let rendered = Arc::new(render(&self.inputs, &identity));
        log_render(workspace_dir, &identity, &rendered, "refresh");
        *state = PromptState {
            fingerprint,
            rendered: Arc::clone(&rendered),
        };
        rendered
    }
}

/// A hash of `(mtime, len)` for every file the identity is read from —
/// missing files hash distinctly from present ones, so a `SOUL.md` created
/// after boot flips it. Cheap enough to run per message.
pub(crate) fn identity_fingerprint(workspace_dir: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    for name in ROOT_IDENTITY_FILES {
        hash_file_stat(&mut hasher, &workspace_dir.join(name));
    }
    hasher.finish()
}

fn hash_file_stat(hasher: &mut DefaultHasher, path: &Path) {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => {
            1u8.hash(hasher);
            meta.len().hash(hasher);
            let modified = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |elapsed| elapsed.as_nanos());
            modified.hash(hasher);
        }
        _ => 0u8.hash(hasher),
    }
}

fn render(inputs: &ChannelPromptInputs, _identity: &ChannelIdentity) -> String {
    let tool_descs: Vec<(&str, &str)> = inputs
        .tool_descs
        .iter()
        .map(|(name, desc)| (name.as_str(), desc.as_str()))
        .collect();
    let identity_override = PromptIdentityOverride { soul_md: None };
    // `channel_name = None`: the runtime wires up several providers at once,
    // so the capability block keeps its platform-agnostic phrasing.
    let mut prompt = build_system_prompt_with_identity(
        &inputs.workspace_dir,
        &inputs.model,
        &tool_descs,
        &inputs.skills,
        inputs.bootstrap_max_chars,
        None,
        identity_override,
        ProjectContextPlacement::Omitted,
    );
    prompt.push_str(&inputs.suffix);
    // Identity after the tool schemas and the access context: a one-line
    // soul followed by ~140k chars of schemas was not honoured by the model,
    // while the same soul near the end of the prompt is (#6027).
    ensure_blank_line(&mut prompt);
    render_project_context(
        &mut prompt,
        &inputs.workspace_dir,
        inputs.bootstrap_max_chars,
        identity_override,
    );
    prompt
}

/// Ends `prompt` with exactly one blank line so the next `##` section is
/// separated the way the builder separates its own.
fn ensure_blank_line(prompt: &mut String) {
    if prompt.is_empty() || prompt.ends_with("\n\n") {
        return;
    }
    if prompt.ends_with('\n') {
        prompt.push('\n');
    } else {
        prompt.push_str("\n\n");
    }
}

/// Names the profile and which files were inlined — never their contents.
fn log_render(workspace_dir: &Path, _identity: &ChannelIdentity, rendered: &str, reason: &str) {
    tracing::info!(
        target: "openhuman::channels",
        workspace = %workspace_dir.display(),
        chars = rendered.chars().count(),
        reason,
        "[channels][prompt] rendered system prompt"
    );
}
