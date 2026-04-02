use crate::openhuman::config::AutocompleteConfig;
use serde::{Deserialize, Serialize};

pub(crate) const MAX_SUGGESTION_CHARS: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteSuggestion {
    pub value: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteStatus {
    pub platform_supported: bool,
    pub enabled: bool,
    pub running: bool,
    pub phase: String,
    pub debounce_ms: u64,
    pub model_id: String,
    pub app_name: Option<String>,
    pub last_error: Option<String>,
    pub updated_at_ms: Option<i64>,
    pub suggestion: Option<AutocompleteSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteStartParams {
    pub debounce_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteStartResult {
    pub started: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteStopParams {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteStopResult {
    pub stopped: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteCurrentParams {
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteCurrentResult {
    pub app_name: Option<String>,
    pub context: String,
    pub suggestion: Option<AutocompleteSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteDebugFocusResult {
    pub app_name: Option<String>,
    pub role: Option<String>,
    pub context: String,
    pub selected_text: Option<String>,
    pub raw_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteAcceptParams {
    pub suggestion: Option<String>,
    /// When true, skip applying text via accessibility (caller already inserted it).
    pub skip_apply: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteAcceptResult {
    pub accepted: bool,
    pub applied: bool,
    pub value: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteSetStyleParams {
    pub enabled: Option<bool>,
    pub debounce_ms: Option<u64>,
    pub max_chars: Option<usize>,
    pub style_preset: Option<String>,
    pub style_instructions: Option<String>,
    pub style_examples: Option<Vec<String>>,
    pub disabled_apps: Option<Vec<String>>,
    pub accept_with_tab: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteSetStyleResult {
    pub config: AutocompleteConfig,
}

#[derive(Debug, Clone)]
pub(crate) struct FocusedTextContext {
    pub(crate) app_name: Option<String>,
    pub(crate) role: Option<String>,
    pub(crate) text: String,
    pub(crate) selected_text: Option<String>,
    pub(crate) raw_error: Option<String>,
    pub(crate) bounds: Option<FocusedElementBounds>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FocusedElementBounds {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}
