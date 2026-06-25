//! UI-facing config operations: browser, screen intelligence, analytics, meet,
//! search, dictation, voice server, onboarding flags.

use serde_json::json;

use crate::openhuman::config::{AutoJoinPolicy, AutoSummarizePolicy, Config};
use crate::openhuman::screen_intelligence;
use crate::rpc::RpcOutcome;

use super::loader::{fallback_workspace_dir, load_config_with_timeout, snapshot_config_json};

#[derive(Debug, Clone, Default)]
pub struct BrowserSettingsPatch {
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct ScreenIntelligenceSettingsPatch {
    pub enabled: Option<bool>,
    pub capture_policy: Option<String>,
    pub policy_mode: Option<String>,
    pub baseline_fps: Option<f32>,
    pub vision_enabled: Option<bool>,
    pub autocomplete_enabled: Option<bool>,
    pub use_vision_model: Option<bool>,
    pub keep_screenshots: Option<bool>,
    pub allowlist: Option<Vec<String>>,
    pub denylist: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct AnalyticsSettingsPatch {
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct MeetSettingsPatch {
    pub auto_orchestrator_handoff: Option<bool>,
    /// Calendar auto-join policy (issue #3511 settings UI).
    pub auto_join_policy: Option<AutoJoinPolicy>,
    /// Post-call auto-summarize policy.
    pub auto_summarize_policy: Option<AutoSummarizePolicy>,
    /// When `true`, the bot joins in listen-only mode (mic muted).
    pub listen_only_default: Option<bool>,
    /// When `true`, backend-bot transcripts are ingested into memory.
    pub ingest_backend_transcripts: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchSettingsPatch {
    /// One of `disabled` | `managed` | `parallel` | `brave` | `querit`.
    /// Empty/unknown values are rejected by `apply_search_settings`.
    /// Runtime fallback to `managed` applies only to persisted/legacy config
    /// values resolved by `SearchConfig::effective_engine()`.
    pub engine: Option<String>,
    /// 1..=20. Clamped silently at apply time.
    pub max_results: Option<usize>,
    /// Per-request timeout in seconds (default 15).
    pub timeout_secs: Option<u64>,
    /// Parallel API key. An empty string clears the stored key.
    pub parallel_api_key: Option<String>,
    /// Brave Search API key. An empty string clears the stored key.
    pub brave_api_key: Option<String>,
    /// Querit API key. An empty string clears the stored key.
    pub querit_api_key: Option<String>,
    /// Websites the assistant may open/read (`web_fetch` / `curl`), as a
    /// host allowlist. Entries are exact hosts (`reuters.com`), which also
    /// match their subdomains, or `"*"` for all public sites. Empty list
    /// blocks all web access. Mirrors `[http_request].allowed_domains`.
    pub allowed_domains: Option<Vec<String>>,
    /// Convenience toggle for the "Allow all sites" switch. `Some(true)`
    /// sets the allowlist to `["*"]`; `Some(false)` drops the wildcard while
    /// keeping any explicit hosts. Applied after `allowed_domains`.
    pub allow_all: Option<bool>,
}

/// Represents a partial update to dictation-related settings.
pub struct DictationSettingsPatch {
    pub enabled: Option<bool>,
    pub hotkey: Option<String>,
    pub activation_mode: Option<String>,
    pub llm_refinement: Option<bool>,
    pub streaming: Option<bool>,
    pub streaming_interval_ms: Option<u64>,
}

/// Represents a partial update to voice server related settings.
pub struct VoiceServerSettingsPatch {
    pub auto_start: Option<bool>,
    pub hotkey: Option<String>,
    pub activation_mode: Option<String>,
    pub skip_cleanup: Option<bool>,
    pub min_duration_secs: Option<f32>,
    pub silence_threshold: Option<f32>,
    pub custom_dictionary: Option<Vec<String>>,
    pub always_on_enabled: Option<bool>,
    pub wake_word: Option<String>,
}

/// Updates the browser-related settings in the configuration.
pub async fn apply_browser_settings(
    config: &mut Config,
    update: BrowserSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(enabled) = update.enabled {
        config.browser.enabled = enabled;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "browser settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies browser settings updates, and saves it.
pub async fn load_and_apply_browser_settings(
    update: BrowserSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_browser_settings(&mut config, update).await
}

/// Updates the screen intelligence settings in the configuration.
pub async fn apply_screen_intelligence_settings(
    config: &mut Config,
    update: ScreenIntelligenceSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(enabled) = update.enabled {
        config.screen_intelligence.enabled = enabled;
    }
    if let Some(capture_policy) = update.capture_policy {
        config.screen_intelligence.capture_policy = capture_policy;
    }
    if let Some(policy_mode) = update.policy_mode {
        config.screen_intelligence.policy_mode = policy_mode;
    }
    if let Some(baseline_fps) = update.baseline_fps {
        config.screen_intelligence.baseline_fps = baseline_fps.clamp(0.2, 30.0);
    }
    if let Some(vision_enabled) = update.vision_enabled {
        config.screen_intelligence.vision_enabled = vision_enabled;
    }
    if let Some(autocomplete_enabled) = update.autocomplete_enabled {
        config.screen_intelligence.autocomplete_enabled = autocomplete_enabled;
    }
    if let Some(use_vision_model) = update.use_vision_model {
        config.screen_intelligence.use_vision_model = use_vision_model;
    }
    if let Some(keep_screenshots) = update.keep_screenshots {
        config.screen_intelligence.keep_screenshots = keep_screenshots;
    }
    if let Some(allowlist) = update.allowlist {
        config.screen_intelligence.allowlist = allowlist;
    }
    if let Some(denylist) = update.denylist {
        config.screen_intelligence.denylist = denylist;
    }

    config.save().await.map_err(|e| e.to_string())?;
    let _ = screen_intelligence::global_engine()
        .apply_config(config.screen_intelligence.clone())
        .await;

    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "screen intelligence settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies screen intelligence settings updates, and saves it.
pub async fn load_and_apply_screen_intelligence_settings(
    update: ScreenIntelligenceSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_screen_intelligence_settings(&mut config, update).await
}

/// Updates the analytics-related settings in the configuration.
pub async fn apply_analytics_settings(
    config: &mut Config,
    update: AnalyticsSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(enabled) = update.enabled {
        config.observability.analytics_enabled = enabled;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "analytics settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies analytics settings updates, and saves it.
pub async fn load_and_apply_analytics_settings(
    update: AnalyticsSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_analytics_settings(&mut config, update).await
}

/// Updates the Google Meet integration settings in the configuration.
pub async fn apply_meet_settings(
    config: &mut Config,
    update: MeetSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(enabled) = update.auto_orchestrator_handoff {
        config.meet.auto_orchestrator_handoff = enabled;
    }
    if let Some(policy) = update.auto_join_policy {
        config.meet.auto_join_policy = policy;
    }
    if let Some(policy) = update.auto_summarize_policy {
        config.meet.auto_summarize_policy = policy;
    }
    if let Some(listen_only) = update.listen_only_default {
        config.meet.listen_only_default = listen_only;
    }
    if let Some(ingest) = update.ingest_backend_transcripts {
        config.meet.ingest_backend_transcripts = ingest;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "meet settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, applies meet settings updates, and saves it.
pub async fn load_and_apply_meet_settings(
    update: MeetSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_meet_settings(&mut config, update).await
}

/// Updates the search engine configuration. Empty API-key strings clear the
/// stored value rather than treat empty-string as "credential present".
pub async fn apply_search_settings(
    config: &mut Config,
    update: SearchSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(engine) = update.engine {
        let trimmed = engine.trim();
        match trimmed {
            "disabled" | "managed" | "parallel" | "brave" | "querit" => {
                config.search.engine = trimmed.to_string();
            }
            other => {
                return Err(format!(
                    "engine must be one of disabled/managed/parallel/brave/querit (got {other:?})"
                ));
            }
        }
    }
    if let Some(n) = update.max_results {
        if !(1..=20).contains(&n) {
            return Err(format!("max_results must be between 1 and 20 (got {n})"));
        }
        config.search.max_results = n;
    }
    if let Some(secs) = update.timeout_secs {
        if !(1..=120).contains(&secs) {
            return Err(format!(
                "timeout_secs must be between 1 and 120 (got {secs})"
            ));
        }
        config.search.timeout_secs = secs;
    }
    if let Some(raw) = update.parallel_api_key {
        let trimmed = raw.trim();
        config.search.parallel.api_key = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    if let Some(raw) = update.brave_api_key {
        let trimmed = raw.trim();
        config.search.brave.api_key = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    if let Some(raw) = update.querit_api_key {
        let trimmed = raw.trim();
        config.search.querit.api_key = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
    }
    let allowlist_touched = update.allowed_domains.is_some() || update.allow_all.is_some();
    let before_count = config.http_request.allowed_domains.len();
    let before_allow_all = config.http_request.allowed_domains.iter().any(|d| d == "*");
    if let Some(domains) = update.allowed_domains {
        let mut cleaned: Vec<String> = domains
            .into_iter()
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty())
            .collect();
        cleaned.sort();
        cleaned.dedup();
        config.http_request.allowed_domains = cleaned;
    }
    if let Some(allow_all) = update.allow_all {
        if allow_all {
            config.http_request.allowed_domains = vec!["*".to_string()];
        } else {
            config.http_request.allowed_domains.retain(|d| d != "*");
        }
    }
    if allowlist_touched {
        let after_count = config.http_request.allowed_domains.len();
        let after_allow_all = config.http_request.allowed_domains.iter().any(|d| d == "*");
        tracing::info!(
            before_count,
            after_count,
            before_allow_all,
            after_allow_all,
            "[config] http_request.allowed_domains updated"
        );
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "search settings saved to {}",
            config.config_path.display()
        )],
    ))
}

pub async fn load_and_apply_search_settings(
    update: SearchSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_search_settings(&mut config, update).await
}

/// Read the current search engine settings (with API keys redacted to a
/// presence boolean so the UI can show "configured" without ever rendering
/// the raw secret).
pub async fn get_search_settings() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let result = serde_json::json!({
        "engine": config.search.requested_engine_str(),
        "effective_engine": match config.search.effective_engine() {
            crate::openhuman::config::SearchEngine::Disabled => "disabled",
            crate::openhuman::config::SearchEngine::Managed => "managed",
            crate::openhuman::config::SearchEngine::Parallel => "parallel",
            crate::openhuman::config::SearchEngine::Brave => "brave",
            crate::openhuman::config::SearchEngine::Querit => "querit",
        },
        "max_results": config.search.max_results,
        "timeout_secs": config.search.timeout_secs,
        "parallel_configured": config.search.parallel.has_key(),
        "brave_configured": config.search.brave.has_key(),
        "querit_configured": config.search.querit.has_key(),
        "allowed_domains": config.http_request.allowed_domains,
        "allow_all": config.http_request.allowed_domains.iter().any(|d| d == "*"),
    });
    Ok(RpcOutcome::new(
        result,
        vec!["search settings read".to_string()],
    ))
}

/// Resolves a workspace onboarding flag, creating or checking its existence.
pub async fn workspace_onboarding_flag_resolve(
    flag_name: Option<String>,
    default_name: &str,
) -> Result<RpcOutcome<bool>, String> {
    let name = flag_name.unwrap_or_else(|| default_name.to_string());
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err("Invalid onboarding flag name".to_string());
    }
    let workspace_dir = match load_config_with_timeout().await {
        Ok(cfg) => cfg.workspace_dir,
        Err(_) => fallback_workspace_dir(),
    };
    workspace_onboarding_flag_exists(workspace_dir, trimmed)
}

/// Checks if a specific onboarding flag file exists in the workspace.
pub fn workspace_onboarding_flag_exists(
    workspace_dir: std::path::PathBuf,
    flag_name: &str,
) -> Result<RpcOutcome<bool>, String> {
    let trimmed = flag_name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err("Invalid onboarding flag name".to_string());
    }
    Ok(RpcOutcome::single_log(
        workspace_dir.join(trimmed).is_file(),
        "onboarding flag checked",
    ))
}

/// Creates or removes an onboarding flag file in the workspace.
pub async fn workspace_onboarding_flag_set(
    flag_name: Option<String>,
    default_name: &str,
    value: bool,
) -> Result<RpcOutcome<bool>, String> {
    let name = flag_name.unwrap_or_else(|| default_name.to_string());
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err("Invalid onboarding flag name".to_string());
    }
    let workspace_dir = match load_config_with_timeout().await {
        Ok(cfg) => cfg.workspace_dir,
        Err(_) => fallback_workspace_dir(),
    };
    let flag_path = workspace_dir.join(trimmed);
    if value {
        if let Some(parent) = flag_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create workspace dir: {e}"))?;
        }
        std::fs::write(&flag_path, "")
            .map_err(|e| format!("Failed to create onboarding flag: {e}"))?;
    } else if flag_path.is_file() {
        std::fs::remove_file(&flag_path)
            .map_err(|e| format!("Failed to remove onboarding flag: {e}"))?;
    }
    Ok(RpcOutcome::single_log(
        flag_path.is_file(),
        "onboarding flag updated",
    ))
}

/// Returns whether the onboarding process has been marked as completed.
pub async fn get_onboarding_completed() -> Result<RpcOutcome<bool>, String> {
    let config = load_config_with_timeout().await?;
    Ok(RpcOutcome::single_log(
        config.onboarding_completed,
        "onboarding_completed read from config",
    ))
}

/// Updates and persists the onboarding completion status.
///
/// On a false→true transition, seeds the recurring morning-briefing
/// cron job via [`crate::openhuman::cron::seed::seed_proactive_agents`].
pub async fn set_onboarding_completed(value: bool) -> Result<RpcOutcome<bool>, String> {
    tracing::debug!(value, "[onboarding] set_onboarding_completed called");
    let mut config = load_config_with_timeout().await?;
    let was_completed = config.onboarding_completed;
    config.onboarding_completed = value;

    config.save().await.map_err(|e| e.to_string())?;

    if value && !was_completed {
        tracing::debug!(
            "[onboarding] false→true transition detected — seeding cron jobs (welcome is renderer-triggered)"
        );
        let seed_config = config.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = crate::openhuman::cron::seed::seed_proactive_agents(&seed_config) {
                tracing::warn!("[onboarding] failed to seed proactive agent cron jobs: {e}");
            }
        });
    } else {
        tracing::debug!(
            was_completed,
            value,
            "[onboarding] no transition — skipping proactive seeding"
        );
    }

    Ok(RpcOutcome::single_log(
        config.onboarding_completed,
        "onboarding_completed saved to config",
    ))
}

/// Reads the "super context" toggle (`context.super_context_enabled`).
///
/// When on, the agent harness runs a mandatory read-only context-collection
/// pass on the first turn of a new thread before the orchestrator LLM runs.
/// Surfaced as the toggle below the chat composer.
pub async fn get_super_context_enabled() -> Result<RpcOutcome<bool>, String> {
    let config = load_config_with_timeout().await?;
    Ok(RpcOutcome::single_log(
        config.context.super_context_enabled,
        "super_context_enabled read from config",
    ))
}

/// Updates and persists the "super context" toggle.
///
/// Read at thread/session construction, so the new value only takes effect
/// for threads started after the change (matches the frozen turn-1 prefix
/// contract).
pub async fn set_super_context_enabled(value: bool) -> Result<RpcOutcome<bool>, String> {
    tracing::debug!(value, "[super_context] set_super_context_enabled called");
    let mut config = load_config_with_timeout().await?;
    config.context.super_context_enabled = value;
    config.save().await.map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        config.context.super_context_enabled,
        "super_context_enabled saved to config",
    ))
}

/// Returns the current dictation settings as a JSON object.
pub async fn get_dictation_settings() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let result = json!({
        "enabled": config.dictation.enabled,
        "hotkey": config.dictation.hotkey,
        "activation_mode": config.dictation.activation_mode,
        "llm_refinement": config.dictation.llm_refinement,
        "streaming": config.dictation.streaming,
        "streaming_interval_ms": config.dictation.streaming_interval_ms,
    });
    Ok(RpcOutcome::new(
        result,
        vec!["dictation settings read".to_string()],
    ))
}

/// Loads configuration, applies dictation settings updates, and saves it.
pub async fn load_and_apply_dictation_settings(
    update: DictationSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    if let Some(enabled) = update.enabled {
        config.dictation.enabled = enabled;
    }
    if let Some(hotkey) = update.hotkey {
        config.dictation.hotkey = hotkey;
    }
    if let Some(mode) = update.activation_mode {
        match mode.as_str() {
            "toggle" => {
                config.dictation.activation_mode =
                    crate::openhuman::config::DictationActivationMode::Toggle;
            }
            "push" => {
                config.dictation.activation_mode =
                    crate::openhuman::config::DictationActivationMode::Push;
            }
            _ => {
                return Err(format!(
                    "invalid activation_mode: {mode} (valid: toggle, push)"
                ))
            }
        }
    }
    if let Some(llm_refinement) = update.llm_refinement {
        config.dictation.llm_refinement = llm_refinement;
    }
    if let Some(streaming) = update.streaming {
        config.dictation.streaming = streaming;
    }
    if let Some(interval) = update.streaming_interval_ms {
        config.dictation.streaming_interval_ms = interval;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(&config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "dictation settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Returns the current voice server settings as a JSON object.
pub async fn get_voice_server_settings() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let result = json!({
        "auto_start": config.voice_server.auto_start,
        "hotkey": config.voice_server.hotkey,
        "activation_mode": config.voice_server.activation_mode,
        "skip_cleanup": config.voice_server.skip_cleanup,
        "min_duration_secs": config.voice_server.min_duration_secs,
        "silence_threshold": config.voice_server.silence_threshold,
        "custom_dictionary": config.voice_server.custom_dictionary,
        "always_on_enabled": config.voice_server.always_on_enabled,
        "wake_word": config.voice_server.wake_word,
    });
    Ok(RpcOutcome::new(
        result,
        vec!["voice server settings read".to_string()],
    ))
}

/// Loads configuration, applies voice server settings updates, and saves it.
pub async fn load_and_apply_voice_server_settings(
    update: VoiceServerSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    if let Some(auto_start) = update.auto_start {
        config.voice_server.auto_start = auto_start;
    }
    if let Some(hotkey) = update.hotkey {
        config.voice_server.hotkey = hotkey;
    }
    if let Some(mode) = update.activation_mode {
        match mode.as_str() {
            "tap" => {
                config.voice_server.activation_mode =
                    crate::openhuman::config::VoiceActivationMode::Tap;
            }
            "push" => {
                config.voice_server.activation_mode =
                    crate::openhuman::config::VoiceActivationMode::Push;
            }
            _ => {
                return Err(format!(
                    "invalid activation_mode: {mode} (valid: tap, push)"
                ))
            }
        }
    }
    if let Some(skip_cleanup) = update.skip_cleanup {
        config.voice_server.skip_cleanup = skip_cleanup;
    }
    if let Some(min_duration_secs) = update.min_duration_secs {
        config.voice_server.min_duration_secs = min_duration_secs.max(0.0);
    }
    if let Some(silence_threshold) = update.silence_threshold {
        config.voice_server.silence_threshold = silence_threshold.max(0.0);
    }
    if let Some(custom_dictionary) = update.custom_dictionary {
        config.voice_server.custom_dictionary = custom_dictionary;
    }
    if let Some(always_on_enabled) = update.always_on_enabled {
        config.voice_server.always_on_enabled = always_on_enabled;
    }
    if let Some(wake_word) = update.wake_word {
        config.voice_server.wake_word = wake_word.trim().to_string();
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(&config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "voice server settings saved to {}",
            config.config_path.display()
        )],
    ))
}
