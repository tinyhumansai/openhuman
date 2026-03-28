use crate::openhuman::config::{AutocompleteConfig, Config};
use crate::openhuman::local_ai;
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
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
}

fn is_text_role(role: Option<&str>) -> bool {
    matches!(
        role.unwrap_or_default(),
        "AXTextArea" | "AXTextField" | "AXSearchField" | "AXComboBox" | "AXEditableText"
    )
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
            task: None,
        }
    }
}

pub struct AutocompleteEngine {
    inner: Mutex<EngineState>,
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
                let _ = engine.try_accept_via_tab().await;
                if last_refresh.elapsed() >= Duration::from_millis(debounce_ms) {
                    if let Err(err) = engine.refresh(None).await {
                        let mut state = engine.inner.lock().await;
                        state.phase = "error".to_string();
                        state.last_error = Some(err);
                        state.updated_at_ms = Some(Utc::now().timestamp_millis());
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
        if let Some(task) = state.task.take() {
            task.abort();
        }
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
        let mut state = self.inner.lock().await;
        state.suggestion = None;
        state.phase = "idle".to_string();
        state.updated_at_ms = Some(Utc::now().timestamp_millis());

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
        if !config.autocomplete.enabled {
            state.running = false;
            if let Some(task) = state.task.take() {
                task.abort();
            }
            state.suggestion = None;
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
            }
        } else {
            focused_text_context()?
        };

        let app_lower = focused.app_name.clone().unwrap_or_default().to_lowercase();
        if config
            .autocomplete
            .disabled_apps
            .iter()
            .any(|needle| !needle.trim().is_empty() && app_lower.contains(needle))
        {
            let mut state = self.inner.lock().await;
            state.app_name = focused.app_name;
            state.context = truncate_tail(&focused.text, config.autocomplete.max_chars);
            state.suggestion = None;
            state.phase = "blocked_app".to_string();
            state.last_error = None;
            state.updated_at_ms = Some(Utc::now().timestamp_millis());
            return Ok(());
        }

        let context = truncate_tail(&focused.text, config.autocomplete.max_chars);
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
        let generated = service
            .inline_complete(
                &config,
                &context,
                &config.autocomplete.style_preset,
                config.autocomplete.style_instructions.as_deref(),
                &config.autocomplete.style_examples,
                Some(36),
            )
            .await
            .unwrap_or_default();

        let suggestion = sanitize_suggestion(&generated);
        let mut state = self.inner.lock().await;
        state.app_name = focused.app_name;
        state.context = context;
        state.updated_at_ms = Some(Utc::now().timestamp_millis());
        if suggestion.is_empty() {
            state.suggestion = None;
            state.phase = "idle".to_string();
            state.last_error = None;
            return Ok(());
        }
        state.suggestion = Some(AutocompleteSuggestion {
            value: suggestion,
            confidence: 0.72,
        });
        state.phase = "ready".to_string();
        state.last_error = None;
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
                let mut state = self.inner.lock().await;
                state.suggestion = None;
                state.phase = "idle".to_string();
                state.updated_at_ms = Some(Utc::now().timestamp_millis());
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
    let script = r#"
      tell application "System Events"
        set frontApp to first application process whose frontmost is true
        set appName to name of frontApp
        set roleValue to "unknown"
        set textValue to ""
        set selectedValue to ""
        set errValue to ""
        set targetRoles to {"AXTextArea", "AXTextField", "AXSearchField", "AXComboBox", "AXEditableText"}

        try
          set focusedElement to value of attribute "AXFocusedUIElement" of frontApp
          try
            set roleValue to value of attribute "AXRole" of focusedElement as text
          end try
          try
            set textValue to value of attribute "AXValue" of focusedElement as text
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
            set childElems to value of attribute "AXChildren" of focusedWindow
            repeat with childElem in childElems
              set childRole to ""
              set childValue to ""
              try
                set childRole to value of attribute "AXRole" of childElem as text
              end try
              if childRole is in targetRoles then
                try
                  set childValue to value of attribute "AXValue" of childElem as text
                end try
                if childValue is "missing value" then set childValue to ""
                if childValue is not "" then
                  set roleValue to childRole
                  set textValue to childValue
                  exit repeat
                end if
              end if
            end repeat
          on error errMsg2 number errNum2
            if errValue is "" then set errValue to "ERROR:" & errNum2 & ":" & errMsg2
          end try
        end if

        if textValue is "" and errValue is "" then
          set errValue to "ERROR:no_text_candidate_found"
        end if

        return appName & "\n" & roleValue & "\n" & textValue & "\n" & selectedValue & "\n" & errValue
      end tell
    "#;

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
    let mut lines = text.lines();
    let app_name = lines
        .next()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let role = lines
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let mut value = lines
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .unwrap_or_default();
    let mut selected_text = lines
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let mut raw_error = lines
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());

    if !is_text_role(role.as_deref()) {
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

#[cfg(target_os = "macos")]
fn apply_text_to_focused_field(text: &str) -> Result<(), String> {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace('\n', " ");
    let script = format!(
        r#"tell application "System Events" to keystroke "{}""#,
        escaped
    );
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to run osascript: {e}"))?;
    if !output.status.success() {
        return Err("failed to apply suggestion to focused text field".to_string());
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
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
}

#[cfg(target_os = "macos")]
const KCG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE: i32 = 0;
#[cfg(target_os = "macos")]
const KVK_TAB: u16 = 48;
