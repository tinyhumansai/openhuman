//! The channel runtime's system prompt: fixed for tests, identity-refreshing
//! in production.
//!
//! Two defects lived in the old `Arc<String>`:
//!
//! - **#6027** — the native channel runtime rendered its prompt from the
//!   workspace-root `SOUL.md` and never consulted the active agent profile,
//!   so a user who picked a personality got it in desktop chat and the stock
//!   voice on every bot-token channel (the managed-DM path goes through
//!   `web_chat` and did apply it).
//! - **#6028** — the prompt was rendered once in `start_channels` and kept
//!   for the life of the process, so a profile switch, a `SOUL.md` edit or
//!   the archivist writing `MEMORY.md` stayed invisible until restart.
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

use crate::openhuman::agent::context::channels_prompt::{
    build_system_prompt_with_identity, render_project_context, ProjectContextPlacement,
    PromptIdentityOverride,
};
use crate::openhuman::agent::profiles::{
    self, AgentProfile, AgentProfileStore, DEFAULT_PROFILE_ID,
};
use crate::openhuman::skills::Workflow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

/// The file `AgentProfileStore` persists the active profile id in
/// (`<workspace>/agent_profiles.json`, see `profiles/store.rs`). Watched so a
/// profile switch made in the desktop UI reaches the next channel turn.
const AGENT_PROFILES_FILE: &str = "agent_profiles.json";

/// Workspace-root files `build_system_prompt` inlines.
const ROOT_IDENTITY_FILES: [&str; 4] = ["SOUL.md", "IDENTITY.md", "PROFILE.md", "MEMORY.md"];

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
pub(crate) struct ChannelIdentity {
    pub(crate) profile_id: String,
    /// The profile's own soul, replacing root `SOUL.md`; `None` renders the root file.
    pub(crate) soul_md: Option<String>,
    /// The profile's own memory, replacing root `MEMORY.md`; `None` renders the root file.
    pub(crate) memory_md: Option<String>,
    /// The workspace file the profile's `soul_md_path` resolves to (safe,
    /// absolute — see `profiles::paths::soul_md_file_path`); watched by the
    /// fingerprint so edits to it are picked up too.
    pub(crate) soul_md_path: Option<PathBuf>,
    /// The profile's `system_prompt_suffix`, rendered as the same
    /// `## Agent profile` block desktop chat appends (trimmed, non-empty).
    pub(crate) prompt_suffix: Option<String>,
}

impl ChannelIdentity {
    /// The identity a workspace has with no usable profile store: the root files.
    fn root() -> Self {
        Self {
            profile_id: DEFAULT_PROFILE_ID.to_string(),
            soul_md: None,
            memory_md: None,
            soul_md_path: None,
            prompt_suffix: None,
        }
    }

    fn for_profile(workspace_dir: &Path, profile: &AgentProfile) -> Self {
        Self {
            profile_id: profile.id.clone(),
            soul_md: profiles::paths::resolve_personality_soul(workspace_dir, profile),
            memory_md: profiles::paths::resolve_personality_memory_md(workspace_dir, profile),
            soul_md_path: profiles::paths::soul_md_file_path(workspace_dir, profile),
            prompt_suffix: profile
                .system_prompt_suffix
                .as_deref()
                .map(str::trim)
                .filter(|suffix| !suffix.is_empty())
                .map(str::to_string),
        }
    }
}

struct PromptState {
    fingerprint: u64,
    profile_id: String,
    soul_md_path: Option<PathBuf>,
    rendered: Arc<String>,
}

/// Shared state behind [`ChannelSystemPrompt::Refreshing`].
pub(crate) struct RefreshingInner {
    inputs: ChannelPromptInputs,
    state: Mutex<PromptState>,
    /// An unreadable profile store is reported once per runtime, not once
    /// per message: the fallback is the root identity, which is what the
    /// runtime rendered before profiles existed.
    store_warned: AtomicBool,
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
                    .field("profile_id", &state.profile_id)
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
        let store_warned = AtomicBool::new(false);
        let identity = resolve_channel_identity_once(&inputs.workspace_dir, &store_warned);
        let fingerprint = identity_fingerprint(
            &inputs.workspace_dir,
            &identity.profile_id,
            identity.soul_md_path.as_deref(),
        );
        let rendered = Arc::new(render(&inputs, &identity));
        log_render(&inputs.workspace_dir, &identity, &rendered, "boot");
        Self::Refreshing(Arc::new(RefreshingInner {
            inputs,
            state: Mutex::new(PromptState {
                fingerprint,
                profile_id: identity.profile_id,
                soul_md_path: identity.soul_md_path,
                rendered,
            }),
            store_warned,
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
        let observed = identity_fingerprint(
            workspace_dir,
            &state.profile_id,
            state.soul_md_path.as_deref(),
        );
        if observed == state.fingerprint {
            return Arc::clone(&state.rendered);
        }

        tracing::debug!(
            target: "openhuman::channels",
            profile_id = %state.profile_id,
            "[channels][prompt] identity fingerprint changed; re-rendering system prompt"
        );
        let identity = resolve_channel_identity_once(workspace_dir, &self.store_warned);
        // Fingerprint against the identity we just rendered — a profile
        // switch changes which files are watched, and pinning the old set
        // would re-render on every message that follows.
        let fingerprint = identity_fingerprint(
            workspace_dir,
            &identity.profile_id,
            identity.soul_md_path.as_deref(),
        );
        let rendered = Arc::new(render(&self.inputs, &identity));
        log_render(workspace_dir, &identity, &rendered, "refresh");
        *state = PromptState {
            fingerprint,
            profile_id: identity.profile_id,
            soul_md_path: identity.soul_md_path,
            rendered: Arc::clone(&rendered),
        };
        rendered
    }
}

/// Resolves the active profile the way desktop chat does per turn
/// (`AgentProfileStore::resolve(None)` → active id → built-in default), then
/// the profile's soul and memory. A store that cannot be read is a warning
/// (once per `warned` flag) and the root identity — a channel runtime must
/// come up regardless.
fn resolve_channel_identity_once(workspace_dir: &Path, warned: &AtomicBool) -> ChannelIdentity {
    match AgentProfileStore::new(workspace_dir.to_path_buf()).resolve(None) {
        Ok((_state, profile)) => {
            warned.store(false, Ordering::Relaxed);
            ChannelIdentity::for_profile(workspace_dir, &profile)
        }
        Err(error) => {
            if !warned.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    target: "openhuman::channels",
                    error = %error,
                    "[channels][prompt] agent profile store unreadable; rendering the workspace-root identity"
                );
            }
            ChannelIdentity::root()
        }
    }
}

/// A hash of `(mtime, len)` for every file the identity is read from —
/// missing files hash distinctly from present ones, so a `MEMORY.md` the
/// archivist writes after boot flips it. Cheap enough to run per message.
pub(crate) fn identity_fingerprint(
    workspace_dir: &Path,
    profile_id: &str,
    soul_md_path: Option<&Path>,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    hash_file_stat(&mut hasher, &workspace_dir.join(AGENT_PROFILES_FILE));
    for name in ROOT_IDENTITY_FILES {
        hash_file_stat(&mut hasher, &workspace_dir.join(name));
    }
    let home = profiles::home::profile_home(workspace_dir, profile_id);
    hash_file_stat(&mut hasher, &home.join("SOUL.md"));
    hash_file_stat(&mut hasher, &home.join("MEMORY.md"));
    if let Some(path) = soul_md_path {
        hash_file_stat(&mut hasher, path);
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

fn render(inputs: &ChannelPromptInputs, identity: &ChannelIdentity) -> String {
    let tool_descs: Vec<(&str, &str)> = inputs
        .tool_descs
        .iter()
        .map(|(name, desc)| (name.as_str(), desc.as_str()))
        .collect();
    let identity_override = PromptIdentityOverride {
        soul_md: identity.soul_md.as_deref(),
        memory_md: identity.memory_md.as_deref(),
    };
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
    // The profile's own instructions come last, as the block desktop chat
    // renders through `AgentProfilePromptSection` (#6027: both paths agree).
    if let Some(suffix) = identity.prompt_suffix.as_deref() {
        ensure_blank_line(&mut prompt);
        prompt.push_str(&profiles::prompt_section::render_agent_profile_block(
            suffix, None,
        ));
        prompt.push_str("\n\n");
    }
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
fn log_render(workspace_dir: &Path, identity: &ChannelIdentity, rendered: &str, reason: &str) {
    let soul = if identity.soul_md.is_some() {
        "profile"
    } else {
        "root"
    };
    let memory = if identity.memory_md.is_some() {
        "profile"
    } else if workspace_dir.join("MEMORY.md").is_file() {
        "root"
    } else {
        "none"
    };
    let suffix = if identity.prompt_suffix.is_some() {
        "profile"
    } else {
        "none"
    };
    tracing::info!(
        target: "openhuman::channels",
        profile_id = %identity.profile_id,
        soul,
        memory,
        suffix,
        chars = rendered.chars().count(),
        reason,
        "[channels][prompt] rendered system prompt"
    );
}

#[cfg(test)]
#[path = "system_prompt_tests.rs"]
mod tests;
