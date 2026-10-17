//! Browser and computer-use config types.

use super::super::defaults;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserComputerUseConfig {
    #[serde(default = "default_browser_computer_use_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_browser_computer_use_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub allow_remote_endpoint: bool,
    #[serde(default)]
    pub window_allowlist: Vec<String>,
    #[serde(default)]
    pub max_coordinate_x: Option<i64>,
    #[serde(default)]
    pub max_coordinate_y: Option<i64>,
}

fn default_browser_computer_use_endpoint() -> String {
    "http://127.0.0.1:8787/v1/actions".into()
}

fn default_browser_computer_use_timeout_ms() -> u64 {
    15_000
}

impl Default for BrowserComputerUseConfig {
    fn default() -> Self {
        Self {
            endpoint: default_browser_computer_use_endpoint(),
            timeout_ms: default_browser_computer_use_timeout_ms(),
            allow_remote_endpoint: false,
            window_allowlist: Vec::new(),
            max_coordinate_x: None,
            max_coordinate_y: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserConfig {
    #[serde(default)]
    pub enabled: bool,
    /// DEPRECATED: the browser tool now shares the unified web-access host list
    /// in `[http_request].allowed_domains` (see `tools::ops::all_tools_with_runtime`).
    /// Still parsed for backward compatibility but no longer gates browser
    /// navigation. Manage allowed hosts via Settings → Search → Allowed websites;
    /// browser allow-all remains gated by `OPENHUMAN_BROWSER_ALLOW_ALL`.
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub session_name: Option<String>,
    #[serde(default = "default_browser_backend")]
    pub backend: String,
    #[serde(default = "default_true")]
    pub native_headless: bool,
    #[serde(default = "default_browser_webdriver_url")]
    pub native_webdriver_url: String,
    #[serde(default)]
    pub native_chrome_path: Option<String>,
    #[serde(default)]
    pub computer_use: BrowserComputerUseConfig,
    /// Run Chrome without a visible window.
    #[serde(default = "default_true")]
    pub headless: bool,
    #[serde(default = "default_viewport_width")]
    pub viewport_width: u32,
    #[serde(default = "default_viewport_height")]
    pub viewport_height: u32,
    #[serde(default)]
    pub chrome_path: Option<String>,
    /// `fresh` creates an isolated disposable Chrome profile; `persistent` uses `profile_path`.
    #[serde(default = "default_profile_mode")]
    pub profile_mode: String,
    #[serde(default)]
    pub profile_path: Option<String>,
    #[serde(default)]
    pub download_dir: Option<String>,
    #[serde(default = "default_max_task_steps")]
    pub max_task_steps: usize,
    #[serde(default = "default_task_timeout_secs")]
    pub task_timeout_secs: u64,
    /// Gated browser action kinds a trusted unattended turn (a cron job, a
    /// background job, or a workflow without `require_approval`) may take
    /// without the interactive host approval it cannot get. Empty, the
    /// default, keeps every such action behind the approval gate. Known names
    /// are [`UNATTENDED_BROWSER_ACTIONS`]; any other entry is ignored with a
    /// warning. Chat, channel and unlabelled turns are never affected.
    #[serde(default)]
    pub unattended_actions: Vec<String>,
    /// Keep, per site, the plan a finished browser task ran and the elements
    /// it found, and hand them to the next task there
    /// (`modules::site_memory_host`). Kept in the workspace only, never in the
    /// agent's memory.
    #[serde(default = "default_true")]
    pub learn_from_tasks: bool,
}

/// Action kinds `[browser] unattended_actions` may name: the gated direct
/// actions, by their TinyComputer wire names, plus `task_step` for a browser
/// task paused at `needs_approval`.
pub const UNATTENDED_BROWSER_ACTIONS: &[&str] = &[
    "click",
    "double_click",
    "fill",
    "type",
    "press",
    "select",
    "check",
    "task_step",
];

fn normalized_action(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

impl BrowserConfig {
    /// Whether `kind` is a known gated action listed in `unattended_actions`.
    /// Says nothing about the turn; callers check its origin separately.
    pub fn allows_unattended(&self, kind: &str) -> bool {
        self.unattended_kind(kind).is_some()
    }

    /// The canonical, static name of `kind` when it is a known gated action
    /// listed in `unattended_actions`. Both sides are trimmed and lowercased.
    /// Being static, it is safe to log whatever the caller passed in.
    pub fn unattended_kind(&self, kind: &str) -> Option<&'static str> {
        let kind = normalized_action(kind);
        let canonical = UNATTENDED_BROWSER_ACTIONS
            .iter()
            .copied()
            .find(|known| *known == kind)?;
        self.unattended_actions
            .iter()
            .any(|listed| normalized_action(listed) == canonical)
            .then_some(canonical)
    }

    /// Entries of `unattended_actions` that name no known action kind. They
    /// allow nothing; the loader reports them so a typo is not silent.
    pub fn unknown_unattended_actions(&self) -> Vec<String> {
        self.unattended_actions
            .iter()
            .filter(|listed| {
                !UNATTENDED_BROWSER_ACTIONS.contains(&normalized_action(listed).as_str())
            })
            .cloned()
            .collect()
    }
}

fn default_viewport_width() -> u32 {
    1280
}
fn default_viewport_height() -> u32 {
    800
}
fn default_profile_mode() -> String {
    "fresh".into()
}
fn default_max_task_steps() -> usize {
    20
}
fn default_task_timeout_secs() -> u64 {
    120
}

fn default_true() -> bool {
    defaults::default_true()
}

fn default_browser_backend() -> String {
    "tinycomputer".into()
}

fn default_browser_webdriver_url() -> String {
    "http://127.0.0.1:9515".into()
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_domains: Vec::new(),
            session_name: None,
            backend: default_browser_backend(),
            native_headless: default_true(),
            native_webdriver_url: default_browser_webdriver_url(),
            native_chrome_path: None,
            computer_use: BrowserComputerUseConfig::default(),
            headless: true,
            viewport_width: default_viewport_width(),
            viewport_height: default_viewport_height(),
            chrome_path: None,
            profile_mode: default_profile_mode(),
            profile_path: None,
            download_dir: None,
            max_task_steps: default_max_task_steps(),
            task_timeout_secs: default_task_timeout_secs(),
            unattended_actions: Vec::new(),
            learn_from_tasks: true,
        }
    }
}

#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
