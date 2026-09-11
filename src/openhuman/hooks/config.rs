//! `hooks.json` — schema, discovery, and layering.
//!
//! The file format is Cursor's, version 1:
//!
//! ```json
//! {
//!   "version": 1,
//!   "hooks": {
//!     "beforeShellExecution": [
//!       { "command": "./scripts/audit.sh", "matcher": "^rm ", "timeout": 10 }
//!     ]
//!   }
//! }
//! ```
//!
//! ## Layering
//!
//! Four layers are read, lowest trust last so that a more specific file cannot
//! *remove* a broader policy — the lists concatenate, they do not override. That
//! is the opposite of how config.toml merges, and it is deliberate: an operator
//! who installs a system-wide deny hook must not be overridden by a repository
//! that ships its own `hooks.json`. Since [`HookOutput::merge`] takes the
//! strictest verdict, concatenation is the safe composition.
//!
//! [`HookOutput::merge`]: super::types::HookOutput::merge

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::types::{normalize_key, HookEvent};

/// The only schema version this loader accepts.
pub const HOOKS_SCHEMA_VERSION: u32 = 1;

/// Filename looked for in every layer directory.
pub const HOOKS_FILE_NAME: &str = "hooks.json";

/// Where a hook definition came from. Surfaced in diagnostics and in the RPC
/// listing so "why is this hook running" has an answer that names a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookLayer {
    /// Machine-wide, operator-managed.
    System,
    /// The user's own `~/.openhuman/hooks.json`.
    User,
    /// The core's workspace directory.
    Workspace,
    /// `<project>/.openhuman/hooks.json` inside the action dir.
    Project,
}

impl HookLayer {
    /// Human-readable label for logs and the RPC listing.
    pub fn as_str(self) -> &'static str {
        match self {
            HookLayer::System => "system",
            HookLayer::User => "user",
            HookLayer::Workspace => "workspace",
            HookLayer::Project => "project",
        }
    }
}

/// How a hook is executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookKind {
    /// Spawn a program, hand it the event JSON on stdin, read a decision from
    /// stdout.
    #[default]
    Command,
    /// Evaluate a natural-language condition with a model, which answers
    /// `{ "ok": bool, "reason": string }`.
    Prompt,
}

/// One configured hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDefinition {
    /// The program to run, or the prompt text for a `prompt` hook.
    pub command: String,
    /// Execution strategy.
    #[serde(default, rename = "type")]
    pub kind: HookKind,
    /// Seconds before the hook is killed. Falls back to the engine default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// Restricts which occurrences of the event reach this hook. See
    /// [`super::matcher`] for the per-event semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    /// Follow-ups this hook may inject before the engine stops honouring them.
    /// `None` in the file means "engine default"; explicit JSON `null` means
    /// unlimited, matching Cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_limit: Option<u32>,
    /// Treat a crashed, missing, or timed-out hook as a denial rather than
    /// letting the action through. Off by default: a broken audit script must
    /// not brick the agent, but a security hook can opt into the other trade.
    #[serde(default, alias = "failClosed")]
    pub fail_closed: bool,
    /// Model override for a `prompt` hook.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Skip this definition without deleting it.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Resolved at load time from the file's own location — never read from
    /// the file, so a `hooks.json` cannot claim to be a more trusted layer.
    #[serde(skip_deserializing)]
    pub layer: Option<HookLayer>,
    /// Directory the hook process runs in: the directory holding its
    /// `hooks.json`. Resolved at load time, never read from the file.
    #[serde(skip_deserializing)]
    pub source_dir: Option<PathBuf>,
}

fn default_true() -> bool {
    true
}

impl HookDefinition {
    /// A short identifier for logs: layer plus the head of the command.
    pub fn label(&self) -> String {
        let layer = self.layer.map(HookLayer::as_str).unwrap_or("unknown");
        let head = self.command.split_whitespace().next().unwrap_or("<empty>");
        format!("{layer}:{head}")
    }
}

/// One parsed `hooks.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksFile {
    /// Schema version. Anything other than [`HOOKS_SCHEMA_VERSION`] is rejected.
    #[serde(default)]
    pub version: u32,
    /// Event name → definitions, in run order.
    #[serde(default)]
    pub hooks: BTreeMap<String, Vec<HookDefinition>>,
}

/// The merged, ready-to-run hook set.
#[derive(Debug, Clone, Default)]
pub struct HookConfig {
    /// Definitions grouped by resolved event, in layer order.
    pub by_event: BTreeMap<HookEvent, Vec<HookDefinition>>,
    /// Files that contributed, for diagnostics.
    pub sources: Vec<PathBuf>,
    /// Non-fatal problems found while loading: unreadable files, unknown event
    /// names, wrong versions. Surfaced rather than swallowed — a hook that
    /// silently never runs is worse than one that reports why.
    pub warnings: Vec<String>,
}

impl HookConfig {
    /// Definitions registered for an event, in run order.
    pub fn for_event(&self, event: HookEvent) -> &[HookDefinition] {
        self.by_event
            .get(&event)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Whether anything at all is configured.
    pub fn is_empty(&self) -> bool {
        self.by_event.values().all(Vec::is_empty)
    }

    /// Total number of configured definitions.
    pub fn len(&self) -> usize {
        self.by_event.values().map(Vec::len).sum()
    }

    /// Parse one file's contents into this config, tagging every definition
    /// with its layer and source directory.
    fn absorb(&mut self, path: &Path, layer: HookLayer, contents: &str) {
        let file: HooksFile = match serde_json::from_str(contents) {
            Ok(file) => file,
            Err(error) => {
                self.warnings
                    .push(format!("{}: invalid JSON: {error}", path.display()));
                return;
            }
        };
        if file.version != HOOKS_SCHEMA_VERSION {
            self.warnings.push(format!(
                "{}: unsupported version {} (expected {HOOKS_SCHEMA_VERSION}); ignored",
                path.display(),
                file.version
            ));
            return;
        }
        let source_dir = path.parent().map(Path::to_path_buf);
        for (key, definitions) in file.hooks {
            let Some(event) = HookEvent::parse(&key) else {
                self.warnings.push(format!(
                    "{}: unknown hook event '{key}'; known events: {}",
                    path.display(),
                    known_events_hint(&key)
                ));
                continue;
            };
            if !event.is_wired() {
                self.warnings.push(format!(
                    "{}: '{event}' is defined but not yet fired by this build;                      its hooks will never run",
                    path.display()
                ));
            }
            let slot = self.by_event.entry(event).or_default();
            for mut definition in definitions {
                if definition.command.trim().is_empty() {
                    self.warnings.push(format!(
                        "{}: hook for '{event}' has an empty command; ignored",
                        path.display()
                    ));
                    continue;
                }
                definition.layer = Some(layer);
                definition.source_dir.clone_from(&source_dir);
                if let Some(problem) = unrunnable_reason(&definition) {
                    self.warnings
                        .push(format!("{}: hook for '{event}' {problem}", path.display()));
                }
                slot.push(definition);
            }
        }
        self.sources.push(path.to_path_buf());
    }
}

/// Report a command-hook script that cannot run, or `None` when it looks fine.
///
/// Only *relative* paths are checked. An absolute path may legitimately be
/// resolved on a machine this config was not loaded on, and a bare name is a
/// `PATH` lookup or a shell built-in — guessing at either would produce a
/// warning for a working hook, which is worse than no warning at all.
fn unrunnable_reason(definition: &HookDefinition) -> Option<String> {
    if definition.kind != HookKind::Command {
        return None;
    }
    let program = definition.command.split_whitespace().next()?;
    if !program.starts_with("./") && !program.starts_with("../") {
        return None;
    }
    let resolved = definition.source_dir.as_ref()?.join(program);
    if !resolved.exists() {
        return Some(format!(
            "points at a missing script: {}",
            resolved.display()
        ));
    }
    if !super::exec::is_executable(&resolved) {
        return Some(format!(
            "points at a script that is not executable (chmod +x): {}",
            resolved.display()
        ));
    }
    None
}

/// Suggest the closest known event name, so a typo reports something useful
/// rather than the full list of eighteen.
fn known_events_hint(key: &str) -> String {
    let normalized = normalize_key(key);
    let closest = HookEvent::ALL.iter().copied().find(|event| {
        let candidate = normalize_key(event.as_str());
        candidate.starts_with(&normalized) || normalized.starts_with(&candidate)
    });
    match closest {
        Some(event) => format!("did you mean '{event}'?"),
        None => HookEvent::ALL
            .iter()
            .map(|event| event.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// The directories searched for a `hooks.json`, lowest trust last.
///
/// `project_dir` is the agent's action root; `workspace_dir` is the core's
/// internal state directory. Both are passed in rather than resolved here so
/// the loader stays testable without touching process globals.
pub fn layer_paths(
    project_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
) -> Vec<(HookLayer, PathBuf)> {
    let mut paths = Vec::new();
    if let Some(system) = system_hooks_dir() {
        paths.push((HookLayer::System, system.join(HOOKS_FILE_NAME)));
    }
    if let Some(home) = dirs::home_dir() {
        paths.push((
            HookLayer::User,
            home.join(".openhuman").join(HOOKS_FILE_NAME),
        ));
    }
    if let Some(workspace) = workspace_dir {
        paths.push((HookLayer::Workspace, workspace.join(HOOKS_FILE_NAME)));
    }
    if let Some(project) = project_dir {
        paths.push((
            HookLayer::Project,
            project.join(".openhuman").join(HOOKS_FILE_NAME),
        ));
    }
    paths
}

#[cfg(target_os = "windows")]
fn system_hooks_dir() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|dir| PathBuf::from(dir).join("OpenHuman"))
}

#[cfg(target_os = "macos")]
fn system_hooks_dir() -> Option<PathBuf> {
    Some(PathBuf::from("/Library/Application Support/OpenHuman"))
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn system_hooks_dir() -> Option<PathBuf> {
    Some(PathBuf::from("/etc/openhuman"))
}

/// Read every layer and merge it into one config.
///
/// A missing file is not a warning — most hosts have none. An unreadable or
/// malformed one is, because that is a hook the author believes is running.
pub fn load(project_dir: Option<&Path>, workspace_dir: Option<&Path>) -> HookConfig {
    let mut config = HookConfig::default();
    for (layer, path) in layer_paths(project_dir, workspace_dir) {
        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                log::debug!(
                    "[hooks] loading {} layer from {}",
                    layer.as_str(),
                    path.display()
                );
                config.absorb(&path, layer, &contents);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => config
                .warnings
                .push(format!("{}: unreadable: {error}", path.display())),
        }
    }
    log::debug!(
        "[hooks] loaded {} definition(s) from {} file(s), {} warning(s)",
        config.len(),
        config.sources.len(),
        config.warnings.len()
    );
    config
}

/// Parse a single file's contents. Exposed for the RPC validate endpoint and
/// for tests, which must not depend on the host's real layer directories.
pub fn parse_one(path: &Path, layer: HookLayer, contents: &str) -> HookConfig {
    let mut config = HookConfig::default();
    config.absorb(path, layer, contents);
    config
}
