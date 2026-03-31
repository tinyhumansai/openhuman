use crate::openhuman::config::{AutocompleteConfig, Config};
use crate::openhuman::local_ai;
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::sync::Mutex as StdMutex;
#[cfg(target_os = "macos")]
use std::{
    fs,
    io::Write,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration, Instant};

const MAX_SUGGESTION_CHARS: usize = 64;

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
struct FocusedTextContext {
    app_name: Option<String>,
    role: Option<String>,
    text: String,
    selected_text: Option<String>,
    raw_error: Option<String>,
    bounds: Option<FocusedElementBounds>,
}

#[derive(Debug, Clone, Copy)]
struct FocusedElementBounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn is_text_role(role: Option<&str>) -> bool {
    matches!(
        role.unwrap_or_default(),
        "AXTextArea" | "AXTextField" | "AXSearchField" | "AXComboBox" | "AXEditableText"
    )
}

fn is_terminal_app(app_name: Option<&str>) -> bool {
    let app = app_name.unwrap_or_default().to_ascii_lowercase();
    [
        "terminal",
        "iterm",
        "wezterm",
        "warp",
        "alacritty",
        "kitty",
        "ghostty",
        "hyper",
        "rio",
    ]
    .iter()
    .any(|needle| app.contains(needle))
}

fn looks_like_terminal_buffer(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let line_count = text.lines().count();
    line_count >= 5
        && (lower.contains("$ ")
            || lower.contains("# ")
            || lower.contains("❯")
            || lower.contains("[1] 0:")
            || lower.contains("tmux")
            || lower.contains("cargo run")
            || lower.contains("git status"))
}

fn is_terminal_noise_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }
    trimmed.starts_with('•')
        || trimmed.starts_with('└')
        || trimmed.starts_with('─')
        || trimmed.starts_with('│')
        || (trimmed.starts_with('[')
            && (trimmed.contains(" 0:") || trimmed.contains("[tmux]") || trimmed.contains("\"⠙")))
}

fn extract_terminal_input_context(text: &str) -> String {
    let mut fallback = String::new();
    for raw_line in text.lines().rev().take(40) {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if fallback.is_empty() && !is_terminal_noise_line(line) {
            fallback = line.to_string();
        }
        if is_terminal_noise_line(line) {
            continue;
        }
        if line.contains("$ ")
            || line.contains("# ")
            || line.contains("❯")
            || line.contains("➜")
            || line.contains("λ")
        {
            return line.to_string();
        }
    }
    fallback
}

struct EngineState {
    running: bool,
    phase: String,
    debounce_ms: u64,
    app_name: Option<String>,
    context: String,
    suggestion: Option<AutocompleteSuggestion>,
    last_error: Option<String>,
    updated_at_ms: Option<i64>,
    last_tab_down: bool,
    last_escape_down: bool,
    last_overlay_signature: Option<String>,
    task: Option<JoinHandle<()>>,
}

impl Default for EngineState {
    fn default() -> Self {
        Self {
            running: false,
            phase: "idle".to_string(),
            debounce_ms: 120,
            app_name: None,
            context: String::new(),
            suggestion: None,
            last_error: None,
            updated_at_ms: None,
            last_tab_down: false,
            last_escape_down: false,
            last_overlay_signature: None,
            task: None,
        }
    }
}

pub struct AutocompleteEngine {
    inner: Mutex<EngineState>,
}

impl Default for AutocompleteEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl AutocompleteEngine {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(EngineState::default()),
        }
    }

    pub async fn status(&self) -> AutocompleteStatus {
        let config = Config::load_or_init()
            .await
            .unwrap_or_else(|_| Config::default());
        let state = self.inner.lock().await;

        AutocompleteStatus {
            platform_supported: cfg!(target_os = "macos"),
            enabled: config.autocomplete.enabled,
            running: state.running,
            phase: state.phase.clone(),
            debounce_ms: state.debounce_ms,
            model_id: config.local_ai.chat_model_id,
            app_name: state.app_name.clone(),
            last_error: state.last_error.clone(),
            updated_at_ms: state.updated_at_ms,
            suggestion: state.suggestion.clone(),
        }
    }

    pub async fn start(
        &self,
        params: AutocompleteStartParams,
    ) -> Result<AutocompleteStartResult, String> {
        if !cfg!(target_os = "macos") {
            return Err("autocomplete is only supported on macOS".to_string());
        }

        let config = Config::load_or_init()
            .await
            .map_err(|e| format!("failed to load config: {e}"))?;
        if !config.autocomplete.enabled {
            return Ok(AutocompleteStartResult { started: false });
        }

        let debounce_ms = params
            .debounce_ms
            .unwrap_or(config.autocomplete.debounce_ms)
            .clamp(50, 2000);

        let mut state = self.inner.lock().await;
        if state.running {
            return Ok(AutocompleteStartResult { started: false });
        }
        state.running = true;
        state.phase = "idle".to_string();
        state.debounce_ms = debounce_ms;
        state.last_error = None;

        let engine = global_engine();
        state.task = Some(tokio::spawn(async move {
            let mut last_refresh = Instant::now() - Duration::from_millis(debounce_ms);
            loop {
                {
                    let state = engine.inner.lock().await;
                    if !state.running {
                        break;
                    }
                }
                let _ = engine.try_reject_via_escape().await;
                let _ = engine.try_accept_via_tab().await;
                if last_refresh.elapsed() >= Duration::from_millis(debounce_ms) {
                    if let Err(err) = engine.refresh(None).await {
                        let error_message = {
                            let mut state = engine.inner.lock().await;
                            state.phase = "error".to_string();
                            state.last_error = Some(err);
                            state.updated_at_ms = Some(Utc::now().timestamp_millis());
                            state.last_error.clone()
                        };
                        if let Some(error_message) = error_message {
                            show_overflow_badge("error", None, Some(&error_message), None, None);
                        }
                    } else {
                        let mut state = engine.inner.lock().await;
                        if state.phase == "error" {
                            state.phase = "idle".to_string();
                        }
                        state.last_error = None;
                    }
                    last_refresh = Instant::now();
                }
                time::sleep(Duration::from_millis(24)).await;
            }
        }));

        Ok(AutocompleteStartResult { started: true })
    }

    pub async fn stop(&self, _params: Option<AutocompleteStopParams>) -> AutocompleteStopResult {
        let mut state = self.inner.lock().await;
        state.running = false;
        state.phase = "idle".to_string();
        state.last_escape_down = false;
        state.last_overlay_signature = None;
        if let Some(task) = state.task.take() {
            task.abort();
        }
        #[cfg(target_os = "macos")]
        let _ = overlay_helper_quit();
        AutocompleteStopResult { stopped: true }
    }

    pub async fn current(
        &self,
        params: Option<AutocompleteCurrentParams>,
    ) -> Result<AutocompleteCurrentResult, String> {
        let context_override = params
            .and_then(|p| p.context)
            .filter(|c| !c.trim().is_empty());
        self.refresh(context_override).await?;
        let state = self.inner.lock().await;
        Ok(AutocompleteCurrentResult {
            app_name: state.app_name.clone(),
            context: state.context.clone(),
            suggestion: state.suggestion.clone(),
        })
    }

    pub async fn debug_focus(&self) -> Result<AutocompleteDebugFocusResult, String> {
        let focused = focused_text_context_verbose()?;
        Ok(AutocompleteDebugFocusResult {
            app_name: focused.app_name,
            role: focused.role,
            context: focused.text,
            selected_text: focused.selected_text,
            raw_error: focused.raw_error,
        })
    }

    pub async fn accept(
        &self,
        params: AutocompleteAcceptParams,
    ) -> Result<AutocompleteAcceptResult, String> {
        let value = if let Some(value) = params.suggestion {
            value
        } else {
            let state = self.inner.lock().await;
            state
                .suggestion
                .as_ref()
                .map(|s| s.value.clone())
                .unwrap_or_default()
        };

        let cleaned = sanitize_suggestion(&value);
        if cleaned.is_empty() {
            return Ok(AutocompleteAcceptResult {
                accepted: false,
                applied: false,
                value: None,
                reason: Some("no suggestion available".to_string()),
            });
        }

        {
            let mut state = self.inner.lock().await;
            state.phase = "accepting".to_string();
        }
        apply_text_to_focused_field(&cleaned)?;
        {
            let mut state = self.inner.lock().await;
            state.suggestion = None;
            state.phase = "idle".to_string();
            state.updated_at_ms = Some(Utc::now().timestamp_millis());
            state.last_overlay_signature = None;
        }
        show_overflow_badge("accepted", Some(&cleaned), None, None, None);

        // Persist acceptance for personalisation (fire-and-forget).
        // Dual-write: KV (UI list) + local docs (semantic search).
        {
            let (ctx, app) = {
                let s = self.inner.lock().await;
                (s.context.clone(), s.app_name.clone())
            };
            let sug = cleaned.clone();
            tokio::spawn(async move {
                crate::openhuman::autocomplete::history::save_accepted_completion(
                    &ctx,
                    &sug,
                    app.as_deref(),
                )
                .await;
                crate::openhuman::autocomplete::history::save_completion_to_local_docs(
                    &ctx,
                    &sug,
                    app.as_deref(),
                )
                .await;
            });
        }

        Ok(AutocompleteAcceptResult {
            accepted: true,
            applied: true,
            value: Some(cleaned),
            reason: None,
        })
    }

    pub async fn set_style(
        &self,
        params: AutocompleteSetStyleParams,
    ) -> Result<AutocompleteSetStyleResult, String> {
        let mut config = Config::load_or_init()
            .await
            .map_err(|e| format!("failed to load config: {e}"))?;
        if let Some(enabled) = params.enabled {
            config.autocomplete.enabled = enabled;
        }
        if let Some(debounce_ms) = params.debounce_ms {
            config.autocomplete.debounce_ms = debounce_ms.clamp(50, 2000);
        }
        if let Some(max_chars) = params.max_chars {
            config.autocomplete.max_chars = max_chars.clamp(64, 2048);
        }
        if let Some(style_preset) = params.style_preset {
            config.autocomplete.style_preset = style_preset.trim().to_string();
        }
        if let Some(style_instructions) = params.style_instructions {
            config.autocomplete.style_instructions = if style_instructions.trim().is_empty() {
                None
            } else {
                Some(style_instructions.trim().to_string())
            };
        }
        if let Some(style_examples) = params.style_examples {
            config.autocomplete.style_examples = style_examples
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .take(8)
                .collect();
        }
        if let Some(disabled_apps) = params.disabled_apps {
            config.autocomplete.disabled_apps = disabled_apps
                .into_iter()
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(accept_with_tab) = params.accept_with_tab {
            config.autocomplete.accept_with_tab = accept_with_tab;
        }
        config.save().await.map_err(|e| e.to_string())?;

        let mut state = self.inner.lock().await;
        state.debounce_ms = config.autocomplete.debounce_ms;
        state.last_tab_down = false;
        state.last_escape_down = false;
        if !config.autocomplete.enabled {
            state.running = false;
            if let Some(task) = state.task.take() {
                task.abort();
            }
            state.suggestion = None;
            state.last_overlay_signature = None;
            #[cfg(target_os = "macos")]
            let _ = overlay_helper_quit();
        }

        Ok(AutocompleteSetStyleResult {
            config: config.autocomplete,
        })
    }

    async fn refresh(&self, context_override: Option<String>) -> Result<(), String> {
        let config = Config::load_or_init()
            .await
            .map_err(|e| format!("failed to load config: {e}"))?;
        if !config.autocomplete.enabled {
            let mut state = self.inner.lock().await;
            state.suggestion = None;
            state.phase = "disabled".to_string();
            return Ok(());
        }
        {
            let mut state = self.inner.lock().await;
            state.phase = "capturing_context".to_string();
        }

        let focused = if let Some(context) = context_override {
            FocusedTextContext {
                app_name: None,
                role: None,
                text: context,
                selected_text: None,
                raw_error: None,
                bounds: None,
            }
        } else {
            let focused = focused_text_context_verbose()?;
            if let Some(err) = focused.raw_error.as_deref() {
                if is_no_text_candidate_error(err) {
                    let mut state = self.inner.lock().await;
                    state.app_name = focused.app_name;
                    state.context = String::new();
                    state.suggestion = None;
                    state.phase = "idle".to_string();
                    state.last_error = None;
                    state.updated_at_ms = Some(Utc::now().timestamp_millis());
                    return Ok(());
                }
                return Err(format!(
                    "focused text unavailable via accessibility api: {err}"
                ));
            }
            focused
        };

        let app_lower = focused.app_name.clone().unwrap_or_default().to_lowercase();
        let is_terminalish = is_terminal_app(focused.app_name.as_deref())
            || looks_like_terminal_buffer(&focused.text);
        let focused_text = if is_terminalish {
            extract_terminal_input_context(&focused.text)
        } else {
            focused.text.clone()
        };
        if config
            .autocomplete
            .disabled_apps
            .iter()
            .any(|needle| !needle.trim().is_empty() && app_lower.contains(needle))
        {
            let mut state = self.inner.lock().await;
            state.app_name = focused.app_name;
            state.context = truncate_tail(&focused_text, config.autocomplete.max_chars);
            state.suggestion = None;
            state.phase = "blocked_app".to_string();
            state.last_error = None;
            state.updated_at_ms = Some(Utc::now().timestamp_millis());
            return Ok(());
        }

        let context = truncate_tail(&focused_text, config.autocomplete.max_chars);
        if context.trim().is_empty() {
            let mut state = self.inner.lock().await;
            state.app_name = focused.app_name;
            state.context = context;
            state.suggestion = None;
            state.phase = "idle".to_string();
            state.updated_at_ms = Some(Utc::now().timestamp_millis());
            return Ok(());
        }

        {
            let mut state = self.inner.lock().await;
            state.phase = "generating".to_string();
        }
        let service = local_ai::global(&config);

        // Build personalised style examples from three sources:
        //  1. Semantically relevant past completions (local doc query)
        //  2. Most recent past completions (KV recency signal / fallback)
        //  3. Static user-configured examples
        // Deduplicated and capped at 8 total.
        let relevant_examples =
            crate::openhuman::autocomplete::history::query_relevant_examples(&context, 4).await;
        let recent_examples =
            crate::openhuman::autocomplete::history::load_recent_examples(4).await;
        let static_examples = config.autocomplete.style_examples.clone();

        let merged_examples: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            let mut v = Vec::new();
            for ex in relevant_examples
                .into_iter()
                .chain(recent_examples)
                .chain(static_examples)
            {
                if seen.insert(ex.clone()) {
                    v.push(ex);
                }
                if v.len() >= 8 {
                    break;
                }
            }
            v
        };

        let generated = service
            .inline_complete(
                &config,
                &context,
                &config.autocomplete.style_preset,
                config.autocomplete.style_instructions.as_deref(),
                &merged_examples,
                Some(36),
            )
            .await
            .unwrap_or_default();

        let suggestion = sanitize_suggestion(&generated);
        let app_name = focused.app_name.clone();
        let mut state = self.inner.lock().await;
        state.app_name = app_name.clone();
        state.context = context;
        state.updated_at_ms = Some(Utc::now().timestamp_millis());
        if suggestion.is_empty() {
            state.suggestion = None;
            state.phase = "idle".to_string();
            state.last_error = None;
            state.last_overlay_signature = None;
            return Ok(());
        }
        state.suggestion = Some(AutocompleteSuggestion {
            value: suggestion.clone(),
            confidence: 0.72,
        });
        state.phase = "ready".to_string();
        state.last_error = None;
        let ready_signature = format!(
            "ready:{}:{}",
            app_name.as_deref().unwrap_or_default(),
            suggestion
        );
        if state.last_overlay_signature.as_deref() != Some(ready_signature.as_str()) {
            state.last_overlay_signature = Some(ready_signature);
            drop(state);
            show_overflow_badge(
                "ready",
                Some(&suggestion),
                None,
                app_name.as_deref(),
                focused.bounds.as_ref(),
            );
            return Ok(());
        }
        Ok(())
    }

    async fn try_accept_via_tab(&self) -> Result<(), String> {
        let accept_with_tab = Config::load_or_init()
            .await
            .map(|cfg| cfg.autocomplete.accept_with_tab)
            .unwrap_or(true);
        if !accept_with_tab {
            let mut state = self.inner.lock().await;
            state.last_tab_down = false;
            return Ok(());
        }

        let is_down = is_tab_key_down();
        let pending = {
            let mut state = self.inner.lock().await;
            let edge = is_down && !state.last_tab_down;
            state.last_tab_down = is_down;
            if !edge {
                None
            } else {
                state.suggestion.as_ref().map(|s| s.value.clone())
            }
        };

        if let Some(suggestion) = pending {
            let cleaned = sanitize_suggestion(&suggestion);
            if !cleaned.is_empty() {
                {
                    let mut state = self.inner.lock().await;
                    state.phase = "accepting".to_string();
                }
                apply_text_to_focused_field(&cleaned)?;
                {
                    let mut state = self.inner.lock().await;
                    state.suggestion = None;
                    state.phase = "idle".to_string();
                    state.updated_at_ms = Some(Utc::now().timestamp_millis());
                    state.last_overlay_signature = None;
                }
                show_overflow_badge("accepted", Some(&cleaned), None, None, None);

                // Persist acceptance for personalisation (fire-and-forget).
                // Dual-write: KV (UI list) + local docs (semantic search).
                {
                    let (ctx, app) = {
                        let s = self.inner.lock().await;
                        (s.context.clone(), s.app_name.clone())
                    };
                    let sug = cleaned.clone();
                    tokio::spawn(async move {
                        crate::openhuman::autocomplete::history::save_accepted_completion(
                            &ctx,
                            &sug,
                            app.as_deref(),
                        )
                        .await;
                        crate::openhuman::autocomplete::history::save_completion_to_local_docs(
                            &ctx,
                            &sug,
                            app.as_deref(),
                        )
                        .await;
                    });
                }
            }
        }

        Ok(())
    }

    async fn try_reject_via_escape(&self) -> Result<(), String> {
        let is_down = is_escape_key_down();
        let rejected = {
            let mut state = self.inner.lock().await;
            let edge = is_down && !state.last_escape_down;
            state.last_escape_down = is_down;
            if !edge || state.suggestion.is_none() {
                None
            } else {
                let value = state.suggestion.as_ref().map(|s| s.value.clone());
                state.suggestion = None;
                state.phase = "idle".to_string();
                state.updated_at_ms = Some(Utc::now().timestamp_millis());
                state.last_overlay_signature = None;
                value
            }
        };
        if let Some(value) = rejected {
            show_overflow_badge("rejected", Some(&value), None, None, None);
        }
        Ok(())
    }
}

pub static AUTOCOMPLETE_ENGINE: Lazy<Arc<AutocompleteEngine>> =
    Lazy::new(|| Arc::new(AutocompleteEngine::new()));

pub fn global_engine() -> Arc<AutocompleteEngine> {
    AUTOCOMPLETE_ENGINE.clone()
}

#[cfg(target_os = "macos")]
static LAST_OVERFLOW_BADGE: Lazy<StdMutex<Option<(String, i64)>>> =
    Lazy::new(|| StdMutex::new(None));

#[cfg(target_os = "macos")]
struct OverlayHelperProcess {
    child: Child,
    stdin: ChildStdin,
}

#[cfg(target_os = "macos")]
static OVERLAY_HELPER_PROCESS: Lazy<StdMutex<Option<OverlayHelperProcess>>> =
    Lazy::new(|| StdMutex::new(None));

fn truncate_tail(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

fn sanitize_suggestion(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or_default().trim();
    let cleaned = first_line
        .trim_matches('"')
        .replace('\t', " ")
        .replace('\r', "")
        .trim()
        .to_string();
    if cleaned.is_empty() {
        return String::new();
    }
    truncate_tail(&cleaned, MAX_SUGGESTION_CHARS)
}

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
fn show_overflow_badge(
    kind: &str,
    suggestion: Option<&str>,
    error: Option<&str>,
    app_name: Option<&str>,
    anchor_bounds: Option<&FocusedElementBounds>,
) {
    #[cfg(target_os = "macos")]
    {
        const READY_THROTTLE_MS: i64 = 1_200;
        let now_ms = Utc::now().timestamp_millis();
        let signature = format!(
            "{}:{}:{}:{}",
            kind,
            app_name.unwrap_or_default(),
            suggestion.unwrap_or_default(),
            error.unwrap_or_default()
        );

        if let Ok(mut guard) = LAST_OVERFLOW_BADGE.lock() {
            if let Some((last_signature, last_ms)) = guard.as_ref() {
                if *last_signature == signature {
                    return;
                }
                if kind == "ready" && (now_ms - *last_ms) < READY_THROTTLE_MS {
                    return;
                }
            }
            *guard = Some((signature, now_ms));
        }

        if kind == "ready" {
            if let (Some(bounds), Some(suggestion_text)) = (anchor_bounds, suggestion) {
                if overlay_helper_show(bounds, suggestion_text).is_ok() {
                    return;
                }
            }
        } else {
            let _ = overlay_helper_hide();
        }

        let title = match kind {
            "ready" => "OpenHuman suggestion",
            "accepted" => "OpenHuman applied",
            "rejected" => "OpenHuman dismissed",
            "error" => "OpenHuman autocomplete error",
            _ => "OpenHuman autocomplete",
        };

        let mut body = match kind {
            "ready" => suggestion.unwrap_or_default().to_string(),
            "accepted" => format!("Inserted: {}", suggestion.unwrap_or_default()),
            "rejected" => "Suggestion dismissed.".to_string(),
            "error" => error.unwrap_or("Autocomplete failed").to_string(),
            _ => suggestion.unwrap_or_default().to_string(),
        };
        if body.trim().is_empty() {
            body = "No suggestion".to_string();
        }
        body = truncate_tail(&body, 140);

        let subtitle = app_name.unwrap_or_default().trim().to_string();
        let escaped_title = escape_osascript_text(title);
        let escaped_body = escape_osascript_text(&body);
        let escaped_subtitle = escape_osascript_text(&subtitle);

        let script = if subtitle.is_empty() {
            format!(
                r#"display notification "{}" with title "{}""#,
                escaped_body, escaped_title
            )
        } else {
            format!(
                r#"display notification "{}" with title "{}" subtitle "{}""#,
                escaped_body, escaped_title, escaped_subtitle
            )
        };

        std::thread::spawn(move || {
            let _ = std::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output();
        });
    }
}

#[cfg(target_os = "macos")]
fn escape_osascript_text(raw: &str) -> String {
    raw.replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace(['\n', '\r'], " ")
}

#[cfg(target_os = "macos")]
fn overlay_helper_show(bounds: &FocusedElementBounds, text: &str) -> Result<(), String> {
    let message = serde_json::json!({
        "type": "show",
        "x": bounds.x,
        "y": bounds.y,
        "w": bounds.width,
        "h": bounds.height,
        "text": truncate_tail(text, 96),
        "ttl_ms": 1100
    })
    .to_string();
    overlay_helper_send_line(&message)
}

#[cfg(target_os = "macos")]
fn overlay_helper_hide() -> Result<(), String> {
    overlay_helper_send_line(r#"{"type":"hide"}"#)
}

#[cfg(target_os = "macos")]
fn overlay_helper_quit() -> Result<(), String> {
    let mut guard = OVERLAY_HELPER_PROCESS
        .lock()
        .map_err(|_| "overlay helper lock poisoned".to_string())?;
    if let Some(mut helper) = guard.take() {
        let _ = helper.stdin.write_all(br#"{"type":"quit"}"#);
        let _ = helper.stdin.write_all(b"\n");
        let _ = helper.stdin.flush();
        let _ = helper.child.kill();
        let _ = helper.child.wait();
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn overlay_helper_send_line(line: &str) -> Result<(), String> {
    ensure_overlay_helper_running()?;
    let mut guard = OVERLAY_HELPER_PROCESS
        .lock()
        .map_err(|_| "overlay helper lock poisoned".to_string())?;
    let Some(helper) = guard.as_mut() else {
        return Err("overlay helper unavailable".to_string());
    };
    helper
        .stdin
        .write_all(line.as_bytes())
        .and_then(|_| helper.stdin.write_all(b"\n"))
        .and_then(|_| helper.stdin.flush())
        .map_err(|e| format!("failed to write overlay helper stdin: {e}"))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_overlay_helper_running() -> Result<(), String> {
    let mut guard = OVERLAY_HELPER_PROCESS
        .lock()
        .map_err(|_| "overlay helper lock poisoned".to_string())?;

    if let Some(helper) = guard.as_mut() {
        if helper
            .child
            .try_wait()
            .map_err(|e| format!("failed to query overlay helper state: {e}"))?
            .is_none()
        {
            return Ok(());
        }
        *guard = None;
    }

    let binary_path = ensure_overlay_helper_binary()?;
    let mut child = Command::new(&binary_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn overlay helper: {e}"))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to capture overlay helper stdin".to_string())?;
    *guard = Some(OverlayHelperProcess { child, stdin });
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_overlay_helper_binary() -> Result<PathBuf, String> {
    let cache_dir = std::env::temp_dir().join("openhuman-autocomplete-overlay");
    fs::create_dir_all(&cache_dir).map_err(|e| format!("failed to create cache dir: {e}"))?;
    let source_path = cache_dir.join("overlay_helper.swift");
    let binary_path = cache_dir.join("overlay_helper_bin");
    let source = overlay_helper_swift_source();

    let needs_write = match fs::read_to_string(&source_path) {
        Ok(existing) => existing != source,
        Err(_) => true,
    };
    if needs_write {
        fs::write(&source_path, source)
            .map_err(|e| format!("failed to write overlay helper source: {e}"))?;
    }

    let needs_compile = needs_write || !binary_path.exists();
    if needs_compile {
        let output = Command::new("xcrun")
            .arg("swiftc")
            .arg("-O")
            .arg(&source_path)
            .arg("-o")
            .arg(&binary_path)
            .output()
            .or_else(|_| {
                Command::new("swiftc")
                    .arg("-O")
                    .arg(&source_path)
                    .arg("-o")
                    .arg(&binary_path)
                    .output()
            })
            .map_err(|e| format!("failed to invoke swiftc: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(format!(
                "failed to compile overlay helper: {}",
                if stderr.is_empty() {
                    "swiftc returned non-zero exit status".to_string()
                } else {
                    stderr
                }
            ));
        }
    }

    Ok(binary_path)
}

#[cfg(target_os = "macos")]
fn overlay_helper_swift_source() -> &'static str {
    r#"import Cocoa
import Foundation

final class OverlayController {
    private var panel: NSPanel?
    private var textField: NSTextField?
    private var hideWorkItem: DispatchWorkItem?

    func show(x: CGFloat, yTop: CGFloat, width: CGFloat, height: CGFloat, text: String, ttlMs: Int) {
        let screen = NSScreen.main ?? NSScreen.screens.first
        let screenHeight = screen?.frame.height ?? 900
        let panelWidth = min(420, max(140, CGFloat(text.count) * 7 + 26))
        let panelHeight: CGFloat = 26
        let originX = x + max(8, min(width - panelWidth - 8, 28))
        let originYTop = yTop + max(5, min(height - panelHeight - 4, 10))
        let originYCocoa = max(6, screenHeight - originYTop - panelHeight)

        if panel == nil {
            let p = NSPanel(
                contentRect: NSRect(x: originX, y: originYCocoa, width: panelWidth, height: panelHeight),
                styleMask: [.borderless, .nonactivatingPanel],
                backing: .buffered,
                defer: false
            )
            p.level = .statusBar
            p.hasShadow = false
            p.isOpaque = false
            p.backgroundColor = .clear
            p.ignoresMouseEvents = true
            p.collectionBehavior = [.canJoinAllSpaces, .transient]

            let content = NSView(frame: NSRect(x: 0, y: 0, width: panelWidth, height: panelHeight))
            content.wantsLayer = true
            content.layer?.cornerRadius = 6
            content.layer?.backgroundColor = NSColor(white: 0.08, alpha: 0.35).cgColor
            p.contentView = content

            let label = NSTextField(labelWithString: text)
            label.frame = NSRect(x: 8, y: 4, width: panelWidth - 12, height: 18)
            label.textColor = NSColor(white: 1.0, alpha: 0.46)
            label.font = NSFont.systemFont(ofSize: 13)
            label.lineBreakMode = .byTruncatingTail
            content.addSubview(label)

            panel = p
            textField = label
        }

        panel?.setFrame(NSRect(x: originX, y: originYCocoa, width: panelWidth, height: panelHeight), display: true)
        panel?.contentView?.frame = NSRect(x: 0, y: 0, width: panelWidth, height: panelHeight)
        textField?.frame = NSRect(x: 8, y: 4, width: panelWidth - 12, height: 18)
        textField?.stringValue = text
        panel?.orderFrontRegardless()

        hideWorkItem?.cancel()
        let work = DispatchWorkItem { [weak self] in
            self?.hide()
        }
        hideWorkItem = work
        DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(max(120, ttlMs)), execute: work)
    }

    func hide() {
        panel?.orderOut(nil)
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let controller = OverlayController()

DispatchQueue.global(qos: .utility).async {
    while let line = readLine() {
        guard let data = line.data(using: .utf8),
              let payload = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let kind = payload["type"] as? String else {
            continue
        }
        if kind == "show" {
            let x = CGFloat((payload["x"] as? NSNumber)?.doubleValue ?? 0)
            let y = CGFloat((payload["y"] as? NSNumber)?.doubleValue ?? 0)
            let w = CGFloat((payload["w"] as? NSNumber)?.doubleValue ?? 0)
            let h = CGFloat((payload["h"] as? NSNumber)?.doubleValue ?? 0)
            let text = (payload["text"] as? String) ?? ""
            let ttl = (payload["ttl_ms"] as? NSNumber)?.intValue ?? 900
            DispatchQueue.main.async {
                controller.show(x: x, yTop: y, width: w, height: h, text: text, ttlMs: ttl)
            }
        } else if kind == "hide" {
            DispatchQueue.main.async {
                controller.hide()
            }
        } else if kind == "quit" {
            DispatchQueue.main.async {
                controller.hide()
                NSApplication.shared.terminate(nil)
            }
            break
        }
    }
}

app.run()
"#
}

fn is_no_text_candidate_error(err: &str) -> bool {
    err.contains("ERROR:no_text_candidate_found")
}

#[cfg(target_os = "macos")]
fn focused_text_context() -> Result<FocusedTextContext, String> {
    let ctx = focused_text_context_verbose()?;
    if let Some(err) = ctx.raw_error.as_ref() {
        return Err(format!(
            "focused text unavailable via accessibility api: {err}"
        ));
    }
    Ok(ctx)
}

#[cfg(target_os = "macos")]
fn focused_text_context_verbose() -> Result<FocusedTextContext, String> {
    let script = r##"
      tell application "System Events"
        set sep to character id 31
        set frontApp to first application process whose frontmost is true
        set appName to name of frontApp
        set roleValue to "unknown"
        set textValue to ""
        set selectedValue to ""
        set errValue to ""
        set posX to ""
        set posY to ""
        set sizeW to ""
        set sizeH to ""
        set targetRoles to {"AXTextArea", "AXTextField", "AXSearchField", "AXComboBox", "AXEditableText"}

        try
          set focusedElement to value of attribute "AXFocusedUIElement" of frontApp
          try
            set roleValue to value of attribute "AXRole" of focusedElement as text
          end try
          try
            set textValue to value of attribute "AXValue" of focusedElement as text
          end try
          try
            set p to value of attribute "AXPosition" of focusedElement
            set posX to item 1 of p as text
            set posY to item 2 of p as text
          end try
          try
            set s to value of attribute "AXSize" of focusedElement
            set sizeW to item 1 of s as text
            set sizeH to item 2 of s as text
          end try
          if textValue is "missing value" then set textValue to ""
          if textValue is "" then
            try
              set selectedValue to value of attribute "AXSelectedText" of focusedElement as text
            end try
            if selectedValue is "missing value" then set selectedValue to ""
            if selectedValue is not "" then set textValue to selectedValue
          end if
          if textValue is "" then
            try
              set textValue to value of attribute "AXTitle" of focusedElement as text
            end try
            if textValue is "missing value" then set textValue to ""
          end if
        on error errMsg number errNum
          set errValue to "ERROR:" & errNum & ":" & errMsg
        end try

        if textValue is "" then
          try
            set focusedWindow to value of attribute "AXFocusedWindow" of frontApp
            set childElems to entire contents of focusedWindow
            set staticPromptValue to ""
            set staticFallbackValue to ""
            repeat with childElem in childElems
              set childRole to ""
              set childValue to ""
              set childSelectedValue to ""
              try
                set childRole to value of attribute "AXRole" of childElem as text
              end try
              if childRole is in targetRoles then
                try
                  set childValue to value of attribute "AXValue" of childElem as text
                end try
                set childPosX to ""
                set childPosY to ""
                set childSizeW to ""
                set childSizeH to ""
                try
                  set cp to value of attribute "AXPosition" of childElem
                  set childPosX to item 1 of cp as text
                  set childPosY to item 2 of cp as text
                end try
                try
                  set cs to value of attribute "AXSize" of childElem
                  set childSizeW to item 1 of cs as text
                  set childSizeH to item 2 of cs as text
                end try
                if childValue is "missing value" then set childValue to ""
                if childValue is "" then
                  try
                    set childSelectedValue to value of attribute "AXSelectedText" of childElem as text
                  end try
                  if childSelectedValue is "missing value" then set childSelectedValue to ""
                  if childSelectedValue is not "" then set childValue to childSelectedValue
                end if
                if childValue is not "" then
                  set roleValue to childRole
                  set textValue to childValue
                  if childPosX is not "" then set posX to childPosX
                  if childPosY is not "" then set posY to childPosY
                  if childSizeW is not "" then set sizeW to childSizeW
                  if childSizeH is not "" then set sizeH to childSizeH
                  exit repeat
                end if
              end if
            end repeat
            if textValue is "" then
              repeat with childElem in childElems
                set childRole to ""
                set childValue to ""
                try
                  set childRole to value of attribute "AXRole" of childElem as text
                end try
                if childRole is "AXStaticText" then
                  try
                    set childValue to value of attribute "AXValue" of childElem as text
                  end try
                  if childValue is "missing value" then set childValue to ""
                  if childValue is not "" then
                    set staticFallbackValue to childValue
                    if childValue contains "$ " or childValue contains "# " or childValue contains "> " then
                      set staticPromptValue to childValue
                    end if
                  end if
                end if
              end repeat
              if staticPromptValue is not "" then
                set roleValue to "AXStaticText"
                set textValue to staticPromptValue
              else if staticFallbackValue is not "" then
                set roleValue to "AXStaticText"
                set textValue to staticFallbackValue
              end if
            end if
          on error errMsg2 number errNum2
            if errValue is "" then set errValue to "ERROR:" & errNum2 & ":" & errMsg2
          end try
        end if

        if textValue is "" and errValue is "" then
          set errValue to "ERROR:no_text_candidate_found"
        end if

        return appName & sep & roleValue & sep & textValue & sep & selectedValue & sep & errValue & sep & posX & sep & posY & sep & sizeW & sep & sizeH
      end tell
    "##;

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to run osascript: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err("unable to query focused text context".to_string());
        }
        return Err(format!("unable to query focused text context: {stderr}"));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim_end_matches(['\r', '\n']);
    let mut segments = trimmed.splitn(9, '\u{1f}');
    let app_name = segments
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let role = segments
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let mut value = segments.next().map(normalize_ax_value).unwrap_or_default();
    let mut selected_text = segments
        .next()
        .map(normalize_ax_value)
        .filter(|s| !s.is_empty());
    let mut raw_error = segments
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let pos_x = segments.next().and_then(parse_ax_number);
    let pos_y = segments.next().and_then(parse_ax_number);
    let size_w = segments.next().and_then(parse_ax_number);
    let size_h = segments.next().and_then(parse_ax_number);

    let allow_terminal_text_value =
        is_terminal_app(app_name.as_deref()) && !value.trim().is_empty();
    if !is_text_role(role.as_deref()) && !allow_terminal_text_value {
        value.clear();
        selected_text = None;
        if raw_error.is_none() {
            raw_error = Some("ERROR:no_text_candidate_found".to_string());
        }
    }

    Ok(FocusedTextContext {
        app_name,
        role,
        text: value,
        selected_text,
        raw_error,
        bounds: match (pos_x, pos_y, size_w, size_h) {
            (Some(x), Some(y), Some(width), Some(height)) if width > 0 && height > 0 => {
                Some(FocusedElementBounds {
                    x,
                    y,
                    width,
                    height,
                })
            }
            _ => None,
        },
    })
}

#[cfg(not(target_os = "macos"))]
fn focused_text_context() -> Result<FocusedTextContext, String> {
    Err("autocomplete is only supported on macOS".to_string())
}

#[cfg(not(target_os = "macos"))]
fn focused_text_context_verbose() -> Result<FocusedTextContext, String> {
    Err("autocomplete is only supported on macOS".to_string())
}

fn normalize_ax_value(raw: &str) -> String {
    let v = raw.trim();
    if v.eq_ignore_ascii_case("missing value") {
        String::new()
    } else {
        v.to_string()
    }
}

fn parse_ax_number(raw: &str) -> Option<i32> {
    let trimmed = normalize_ax_value(raw);
    if trimmed.is_empty() {
        return None;
    }
    let cleaned = trimmed.replace(',', ".");
    cleaned.parse::<f64>().ok().map(|v| v.round() as i32)
}

#[cfg(target_os = "macos")]
fn apply_text_to_focused_field(text: &str) -> Result<(), String> {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace('\n', " ");
    let script = format!(
        r##"
tell application "System Events"
  set frontApp to first application process whose frontmost is true
  set focusedElement to value of attribute "AXFocusedUIElement" of frontApp
  set currentValue to ""
  try
    set currentValue to value of attribute "AXValue" of focusedElement as text
  end try
  if currentValue is "missing value" then set currentValue to ""
  if currentValue is "" then
    try
      set currentValue to value of attribute "AXSelectedText" of focusedElement as text
    end try
    if currentValue is "missing value" then set currentValue to ""
  end if
  set value of attribute "AXValue" of focusedElement to (currentValue & "{}")
end tell
"##,
        escaped
    );
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to run osascript: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err("failed to apply suggestion to focused text field".to_string());
        }
        return Err(format!(
            "failed to apply suggestion to focused text field: {stderr}"
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn apply_text_to_focused_field(_text: &str) -> Result<(), String> {
    Err("autocomplete is only supported on macOS".to_string())
}

#[cfg(target_os = "macos")]
fn is_tab_key_down() -> bool {
    unsafe { CGEventSourceKeyState(KCG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE, KVK_TAB) }
}

#[cfg(not(target_os = "macos"))]
fn is_tab_key_down() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn is_escape_key_down() -> bool {
    unsafe { CGEventSourceKeyState(KCG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE, KVK_ESCAPE) }
}

#[cfg(not(target_os = "macos"))]
fn is_escape_key_down() -> bool {
    false
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
}

#[cfg(target_os = "macos")]
const KCG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE: i32 = 0;
#[cfg(target_os = "macos")]
const KVK_TAB: u16 = 48;
#[cfg(target_os = "macos")]
const KVK_ESCAPE: u16 = 53;
