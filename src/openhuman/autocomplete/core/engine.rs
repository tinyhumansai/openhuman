use crate::openhuman::config::Config;
use crate::openhuman::local_ai;
use chrono::Utc;
use once_cell::sync::Lazy;
use std::sync::{Arc, Once};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration, Instant};

use super::focus::{
    apply_text_to_focused_field, focused_text_context_verbose, is_escape_key_down, is_tab_key_down,
    validate_focused_target,
};
use super::overlay::{overlay_helper_quit, show_overflow_badge};
use super::terminal::{
    extract_terminal_input_context, is_terminal_app, looks_like_terminal_buffer,
};
use super::text::{is_no_text_candidate_error, sanitize_suggestion, truncate_tail};
use super::types::{
    AutocompleteAcceptParams, AutocompleteAcceptResult, AutocompleteCurrentParams,
    AutocompleteCurrentResult, AutocompleteDebugFocusResult, AutocompleteSetStyleParams,
    AutocompleteSetStyleResult, AutocompleteStartParams, AutocompleteStartResult,
    AutocompleteStatus, AutocompleteStopParams, AutocompleteStopResult, AutocompleteSuggestion,
    FocusedTextContext,
};

const REFRESH_TIMEOUT_SECS: u64 = 120;

struct EngineState {
    running: bool,
    phase: String,
    debounce_ms: u64,
    app_name: Option<String>,
    /// AXRole of the text element when the suggestion was generated.
    target_role: Option<String>,
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
            target_role: None,
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

        // Kick off Swift helper compilation in the background so the first
        // suggestion request does not stall waiting for `swiftc`.
        static PRECOMPILE_ONCE: Once = Once::new();
        PRECOMPILE_ONCE.call_once(|| {
            crate::openhuman::accessibility::precompile_helper_background();
        });

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
                let current_debounce_ms = {
                    let state = engine.inner.lock().await;
                    if !state.running {
                        break;
                    }
                    state.debounce_ms
                };
                let _ = engine.try_reject_via_escape().await;
                let _ = engine.try_accept_via_tab().await;
                if last_refresh.elapsed() >= Duration::from_millis(current_debounce_ms) {
                    let pre_refresh_snapshot = {
                        let state = engine.inner.lock().await;
                        (
                            state.context.clone(),
                            state.app_name.clone(),
                            state.target_role.clone(),
                        )
                    };
                    let refresh_result = time::timeout(
                        Duration::from_secs(REFRESH_TIMEOUT_SECS),
                        engine.refresh(None),
                    )
                    .await;
                    match refresh_result {
                        Ok(Err(err)) => {
                            let error_message = {
                                let mut state = engine.inner.lock().await;
                                state.phase = "error".to_string();
                                state.last_error = Some(err);
                                state.updated_at_ms = Some(Utc::now().timestamp_millis());
                                state.last_error.clone()
                            };
                            if let Some(error_message) = error_message {
                                let app_lower = engine
                                    .inner
                                    .lock()
                                    .await
                                    .app_name
                                    .clone()
                                    .unwrap_or_default()
                                    .to_lowercase();
                                if !app_lower.contains("openhuman") {
                                    show_overflow_badge(
                                        "error",
                                        None,
                                        Some(&error_message),
                                        None,
                                        None,
                                        700,
                                    );
                                }
                            }
                        }
                        Ok(Ok(())) => {
                            let mut state = engine.inner.lock().await;
                            if state.phase == "error" {
                                state.phase = "idle".to_string();
                            }
                            state.last_error = None;
                        }
                        Err(_elapsed) => {
                            log::warn!(
                                "[autocomplete] refresh timed out after {}s, skipping",
                                REFRESH_TIMEOUT_SECS
                            );
                            let mut state = engine.inner.lock().await;
                            let post_refresh_snapshot = (
                                state.context.clone(),
                                state.app_name.clone(),
                                state.target_role.clone(),
                            );
                            if pre_refresh_snapshot != post_refresh_snapshot
                                && state.suggestion.is_some()
                            {
                                log::warn!(
                                    "[autocomplete] clearing stale suggestion after timeout due to metadata drift: pre=({:?},{:?},{:?}) post=({:?},{:?},{:?})",
                                    pre_refresh_snapshot.0,
                                    pre_refresh_snapshot.1,
                                    pre_refresh_snapshot.2,
                                    post_refresh_snapshot.0,
                                    post_refresh_snapshot.1,
                                    post_refresh_snapshot.2
                                );
                                state.suggestion = None;
                                state.last_overlay_signature = None;
                            }
                            state.phase = "idle".to_string();
                            state.last_error =
                                Some(format!("refresh timed out after {}s", REFRESH_TIMEOUT_SECS));
                            state.updated_at_ms = Some(Utc::now().timestamp_millis());
                        }
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
        state.last_error = None;
        state.suggestion = None;
        state.context = String::new();
        state.app_name = None;
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

        let should_apply = !params.skip_apply.unwrap_or(false);

        {
            let mut state = self.inner.lock().await;
            state.phase = "accepting".to_string();
        }
        if should_apply {
            // Validate the focused element still matches before inserting.
            let (expected_app, expected_role) = {
                let state = self.inner.lock().await;
                (state.app_name.clone(), state.target_role.clone())
            };
            let apply_result = (|| -> Result<(), String> {
                #[cfg(target_os = "macos")]
                validate_focused_target(expected_app.as_deref(), expected_role.as_deref())?;
                apply_text_to_focused_field(&cleaned)?;
                Ok(())
            })();
            if let Err(e) = apply_result {
                let mut state = self.inner.lock().await;
                state.phase = if state.suggestion.is_some() {
                    "ready".to_string()
                } else {
                    "idle".to_string()
                };
                state.last_error = Some(e.clone());
                state.updated_at_ms = Some(Utc::now().timestamp_millis());
                return Ok(AutocompleteAcceptResult {
                    accepted: false,
                    applied: false,
                    value: None,
                    reason: Some(format!("accept aborted: {e}")),
                });
            }
        }
        {
            let mut state = self.inner.lock().await;
            state.suggestion = None;
            state.phase = "idle".to_string();
            state.last_error = None;
            state.updated_at_ms = Some(Utc::now().timestamp_millis());
            state.last_overlay_signature = None;
        }
        if should_apply {
            show_overflow_badge("accepted", Some(&cleaned), None, None, None, 700);
        }

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
            applied: should_apply,
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
        if let Some(overlay_ttl_ms) = params.overlay_ttl_ms {
            config.autocomplete.overlay_ttl_ms = overlay_ttl_ms.clamp(300, 10_000);
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
        let is_in_app = context_override.is_some();
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
                app_name: Some("OpenHuman".to_string()),
                role: None,
                text: context,
                selected_text: None,
                raw_error: None,
                bounds: None,
            }
        } else {
            let focused = focused_text_context_verbose()?;
            if let Some(err) = focused.raw_error.as_deref() {
                if is_no_text_candidate_error(err) || err.contains("ERROR:-1728") {
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

        // When OpenHuman itself is focused AND this is the background engine loop,
        // skip AX-based refresh — the in-app React polling handles suggestions.
        // When is_in_app (context_override provided), we still want inference to run.
        if !is_in_app && app_lower.contains("openhuman") {
            let mut state = self.inner.lock().await;
            state.app_name = focused.app_name;
            state.phase = "idle".to_string();
            state.last_error = None;
            state.updated_at_ms = Some(Utc::now().timestamp_millis());
            return Ok(());
        }

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

        // Short-circuit: if context, frontmost app, AND role unchanged and we already have a suggestion, skip inference.
        {
            let mut state = self.inner.lock().await;
            if state.context == context
                && state.app_name == focused.app_name
                && state.target_role == focused.role
                && state.suggestion.is_some()
            {
                log::debug!("[autocomplete] context unchanged, returning cached suggestion");
                return Ok(());
            }
            // Refresh metadata so try_accept_via_tab() sees current values
            state.app_name = focused.app_name.clone();
            state.target_role = focused.role.clone();
            state.updated_at_ms = Some(Utc::now().timestamp_millis());
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
            .await?;

        let suggestion = sanitize_suggestion(&generated);
        let app_name = focused.app_name.clone();
        let target_role = focused.role.clone();
        let mut state = self.inner.lock().await;
        state.app_name = app_name.clone();
        state.target_role = target_role;
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
            // TODO: surface real confidence when local_ai::inline_complete returns it.
            confidence: 0.0,
        });
        state.phase = "ready".to_string();
        state.last_error = None;
        let ready_signature = format!(
            "ready:{}:{}",
            app_name.as_deref().unwrap_or_default(),
            suggestion
        );
        if !is_in_app && state.last_overlay_signature.as_deref() != Some(ready_signature.as_str()) {
            state.last_overlay_signature = Some(ready_signature);
            let overlay_ttl_ms = config.autocomplete.overlay_ttl_ms;
            drop(state);
            show_overflow_badge(
                "ready",
                Some(&suggestion),
                None,
                app_name.as_deref(),
                focused.bounds.as_ref(),
                overlay_ttl_ms,
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

        // Skip AX-based Tab accept when OpenHuman itself is focused —
        // the in-app React handler manages insertion directly.
        {
            let state = self.inner.lock().await;
            let app = state.app_name.as_deref().unwrap_or_default().to_lowercase();
            if app.contains("openhuman") {
                return Ok(());
            }
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
                // Validate the focused element still matches before inserting.
                let (expected_app, expected_role) = {
                    let state = self.inner.lock().await;
                    (state.app_name.clone(), state.target_role.clone())
                };
                #[cfg(target_os = "macos")]
                if let Err(e) =
                    validate_focused_target(expected_app.as_deref(), expected_role.as_deref())
                {
                    log::warn!("[autocomplete] tab-accept aborted: {e}");
                    let mut state = self.inner.lock().await;
                    state.phase = if state.suggestion.is_some() {
                        "ready".to_string()
                    } else {
                        "idle".to_string()
                    };
                    state.last_error = Some(e);
                    state.updated_at_ms = Some(Utc::now().timestamp_millis());
                    return Ok(());
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
                    state.last_error = None;
                    state.updated_at_ms = Some(Utc::now().timestamp_millis());
                    state.last_overlay_signature = None;
                }
                {
                    let app_lower = self
                        .inner
                        .lock()
                        .await
                        .app_name
                        .clone()
                        .unwrap_or_default()
                        .to_lowercase();
                    if !app_lower.contains("openhuman") {
                        show_overflow_badge("accepted", Some(&cleaned), None, None, None, 700);
                    }
                }

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
            let app_lower = self
                .inner
                .lock()
                .await
                .app_name
                .clone()
                .unwrap_or_default()
                .to_lowercase();
            if !app_lower.contains("openhuman") {
                show_overflow_badge("rejected", Some(&value), None, None, None, 700);
            }
        }
        Ok(())
    }
}

pub static AUTOCOMPLETE_ENGINE: Lazy<Arc<AutocompleteEngine>> =
    Lazy::new(|| Arc::new(AutocompleteEngine::new()));

pub fn global_engine() -> Arc<AutocompleteEngine> {
    AUTOCOMPLETE_ENGINE.clone()
}
