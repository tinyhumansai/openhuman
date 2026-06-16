//! Public output types and shared constants for smart_walk.

pub(crate) const SMART_WALK_TEMP: f64 = 0.2;
pub(crate) const HARD_MAX_TURNS: usize = 25;
pub(crate) const MAX_EVIDENCE_ITEMS: usize = 30;
pub(crate) const MAX_KEYWORD_RESULTS: usize = 15;
pub(crate) const MAX_FILE_READ_BYTES: usize = 8000;

pub(crate) fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

// ── Public output types ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SmartWalkOptions {
    pub max_turns: usize,
    pub namespace: String,
    /// Provider string override (e.g. "deepseek:deepseek-chat").
    pub model: Option<String>,
    /// Content root override. Defaults to config.memory_tree_content_root().
    pub content_root: Option<std::path::PathBuf>,
}

impl Default for SmartWalkOptions {
    fn default() -> Self {
        Self {
            max_turns: 12,
            namespace: "default".into(),
            model: None,
            content_root: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartWalkStopReason {
    Answered,
    MaxTurnsReached,
    LlmGaveUp,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct SmartWalkStep {
    pub turn: usize,
    pub action: String,
    pub args_summary: String,
    pub result_preview: String,
}

#[derive(Debug, Clone)]
pub struct Evidence {
    pub source_path: String,
    pub snippet: String,
    pub relevance: String,
}

#[derive(Debug, Clone)]
pub struct SmartWalkOutcome {
    pub answer: String,
    pub evidence: Vec<Evidence>,
    pub trace: Vec<SmartWalkStep>,
    pub turns_used: usize,
    pub stopped_reason: SmartWalkStopReason,
}
