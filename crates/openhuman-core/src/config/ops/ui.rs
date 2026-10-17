//! UI-facing config operations: browser, analytics, dictation, voice server,
//! onboarding flags. Search settings live in `search.rs`.

use crate::core::runtime::DispatchCtx;
use serde_json::json;

use crate::config::Config;
use crate::core::Outcome;

use super::loader::{fallback_workspace_dir, snapshot_config_json};

#[derive(Debug, Clone, Default)]
pub struct BrowserSettingsPatch {
    pub enabled: Option<bool>,
    pub backend: Option<String>,
    pub headless: Option<bool>,
    pub viewport_width: Option<u32>,
    pub viewport_height: Option<u32>,
    pub chrome_path: Option<String>,
    pub profile_mode: Option<String>,
    pub profile_path: Option<String>,
    pub download_dir: Option<String>,
    pub max_task_steps: Option<usize>,
    pub task_timeout_secs: Option<u64>,
    pub learn_from_tasks: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct AnalyticsSettingsPatch {
    pub enabled: Option<bool>,
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
    /// Hosted STT engine name — `"backend"` / `"elevenlabs"` / `"openai"`
    /// (the `"cloud"` / `"openhuman"` aliases also resolve to backend).
    pub stt_engine: Option<String>,
}

/// Updates the browser-related settings in the configuration.
pub async fn apply_browser_settings(
    config: &mut Config,
    update: BrowserSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let normalized_backend = update
        .backend
        .as_deref()
        .map(normalize_browser_backend)
        .transpose()?;
    let mut browser = config.browser.clone();

    if let Some(enabled) = update.enabled {
        browser.enabled = enabled;
    }
    if let Some(backend) = normalized_backend {
        browser.backend = backend;
    }
    if let Some(headless) = update.headless {
        browser.headless = headless;
    }
    if let Some(width) = update.viewport_width {
        if !(320..=3840).contains(&width) {
            return Err("viewport_width must be 320..=3840".into());
        }
        browser.viewport_width = width;
    }
    if let Some(height) = update.viewport_height {
        if !(240..=2160).contains(&height) {
            return Err("viewport_height must be 240..=2160".into());
        }
        browser.viewport_height = height;
    }
    if let Some(path) = update.chrome_path {
        browser.chrome_path = nonempty(path);
    }
    if let Some(mode) = update.profile_mode {
        if mode != "fresh" && mode != "persistent" {
            return Err("profile_mode must be fresh or persistent".into());
        }
        browser.profile_mode = mode;
    }
    if let Some(path) = update.profile_path {
        browser.profile_path = nonempty(path);
    }
    if let Some(path) = update.download_dir {
        browser.download_dir = nonempty(path);
    }
    if browser.profile_mode == "persistent" && browser.profile_path.is_none() {
        return Err("persistent profile requires profile_path".into());
    }
    if let Some(steps) = update.max_task_steps {
        if !(1..=100).contains(&steps) {
            return Err("max_task_steps must be 1..=100".into());
        }
        browser.max_task_steps = steps;
    }
    if let Some(timeout) = update.task_timeout_secs {
        if !(5..=600).contains(&timeout) {
            return Err("task_timeout_secs must be 5..=600".into());
        }
        browser.task_timeout_secs = timeout;
    }
    if let Some(learn) = update.learn_from_tasks {
        browser.learn_from_tasks = learn;
    }
    config.browser = browser;
    config.save().await.map_err(|e| e.to_string())?;
    // Tools built before the switch hold the config they were built with.
    #[cfg(feature = "modules")]
    if let Some(learn) = update.learn_from_tasks {
        crate::modules::site_memory_host::note_switch(&config.workspace_dir, learn);
    }
    let snapshot = snapshot_config_json(config)?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "browser settings saved to {}",
            config.config_path.display()
        )],
    ))
}

fn nonempty(value: String) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn normalize_browser_backend(raw: &str) -> Result<String, String> {
    let key = raw.trim().to_ascii_lowercase().replace('-', "_");
    match key.as_str() {
        // TinyBrowser was folded into TinyComputer; old values migrate.
        "tinycomputer" | "tiny_computer" | "tinybrowser" | "tiny_browser" => {
            Ok("tinycomputer".to_string())
        }
        "agent_browser" | "agentbrowser" => Ok("agent_browser".to_string()),
        "playwright" => Ok("playwright".to_string()),
        "rust_native" | "native" => Ok("rust_native".to_string()),
        "computer_use" | "computeruse" => Ok("computer_use".to_string()),
        "auto" => Ok("auto".to_string()),
        _ => Err(format!(
            "Unsupported browser backend '{raw}'. Use agent_browser, playwright, rust_native, computer_use, or auto"
        )),
    }
}

/// Loads the configuration, applies browser settings updates, and saves it.
pub async fn load_and_apply_browser_settings(
    ctx: &DispatchCtx,
    update: BrowserSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    apply_browser_settings(&mut config, update).await
}

/// Updates the analytics-related settings in the configuration.
pub async fn apply_analytics_settings(
    config: &mut Config,
    update: AnalyticsSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    if let Some(enabled) = update.enabled {
        config.observability.analytics_enabled = enabled;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(config)?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "analytics settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Sets the user's time zone (`None`, or a blank name, follows the device
/// again). The name must be an IANA zone; it is stored in its canonical form.
pub async fn apply_user_timezone(
    config: &mut Config,
    timezone: Option<String>,
) -> Result<Outcome<serde_json::Value>, String> {
    let requested = timezone.as_deref().map(str::trim).filter(|z| !z.is_empty());
    let next = match requested {
        None => None,
        Some(zone) => Some(
            crate::config::normalize_time_zone(zone)
                .ok_or_else(|| format!("{zone:?} is not an IANA time zone (e.g. Asia/Kolkata)"))?,
        ),
    };
    // Build the reply before committing, so nothing after the save can fail
    // and report a stored change as failed; a failed save restores the
    // caller's config to what is on disk.
    let previous = std::mem::replace(&mut config.user_timezone, next);
    let snapshot = match snapshot_config_json(config) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            config.user_timezone = previous;
            return Err(error);
        }
    };
    if let Err(error) = config.save().await {
        config.user_timezone = previous;
        return Err(error.to_string());
    }
    let chosen = config.user_timezone.as_deref().unwrap_or("device");
    log::info!("[config] user time zone set: {chosen}");
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "time zone saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Loads the configuration, sets the user's time zone, and saves it.
pub async fn load_and_apply_user_timezone(
    ctx: &DispatchCtx,
    timezone: Option<String>,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    apply_user_timezone(&mut config, timezone).await
}

/// The user's time zone setting, the device's zone, and the one in effect.
pub fn user_timezone_json(config: &Config) -> serde_json::Value {
    json!({
        "timezone": config.user_timezone,
        "device": crate::config::device_time_zone(),
        "effective": config.time_zone(),
    })
}

/// Loads the configuration, applies analytics settings updates, and saves it.
pub async fn load_and_apply_analytics_settings(
    ctx: &DispatchCtx,
    update: AnalyticsSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    apply_analytics_settings(&mut config, update).await
}

/// Resolves a workspace onboarding flag, creating or checking its existence.
pub async fn workspace_onboarding_flag_resolve(
    ctx: &DispatchCtx,
    flag_name: Option<String>,
    default_name: &str,
) -> Result<Outcome<bool>, String> {
    let name = flag_name.unwrap_or_else(|| default_name.to_string());
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err("Invalid onboarding flag name".to_string());
    }
    let workspace_dir = match ctx.load_config().await {
        Ok(cfg) => cfg.workspace_dir,
        Err(_) => fallback_workspace_dir(),
    };
    workspace_onboarding_flag_exists(workspace_dir, trimmed)
}

/// Checks if a specific onboarding flag file exists in the workspace.
pub fn workspace_onboarding_flag_exists(
    workspace_dir: std::path::PathBuf,
    flag_name: &str,
) -> Result<Outcome<bool>, String> {
    let trimmed = flag_name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err("Invalid onboarding flag name".to_string());
    }
    Ok(Outcome::single_log(
        workspace_dir.join(trimmed).is_file(),
        "onboarding flag checked",
    ))
}

/// Creates or removes an onboarding flag file in the workspace.
pub async fn workspace_onboarding_flag_set(
    ctx: &DispatchCtx,
    flag_name: Option<String>,
    default_name: &str,
    value: bool,
) -> Result<Outcome<bool>, String> {
    let name = flag_name.unwrap_or_else(|| default_name.to_string());
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        return Err("Invalid onboarding flag name".to_string());
    }
    let workspace_dir = match ctx.load_config().await {
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
    Ok(Outcome::single_log(
        flag_path.is_file(),
        "onboarding flag updated",
    ))
}

/// Returns whether the onboarding process has been marked as completed.
pub async fn get_onboarding_completed(ctx: &DispatchCtx) -> Result<Outcome<bool>, String> {
    let config = ctx.load_config().await?;
    Ok(Outcome::single_log(
        config.onboarding_completed,
        "onboarding_completed read from config",
    ))
}

/// Updates and persists the onboarding completion status.
///
/// On a false→true transition, seeds the recurring morning-briefing
/// cron job via [`crate::cron::seed::seed_proactive_agents`].
pub async fn set_onboarding_completed(
    ctx: &DispatchCtx,
    value: bool,
) -> Result<Outcome<bool>, String> {
    tracing::debug!(value, "[onboarding] set_onboarding_completed called");
    let mut config = ctx.load_config().await?;
    let was_completed = config.onboarding_completed;
    config.onboarding_completed = value;

    config.save().await.map_err(|e| e.to_string())?;

    if value && !was_completed {
        tracing::debug!(
            "[onboarding] false→true transition detected — seeding cron jobs (welcome is renderer-triggered)"
        );
        let seed_config = config.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = crate::cron::seed::seed_proactive_agents(&seed_config) {
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

    Ok(Outcome::single_log(
        config.onboarding_completed,
        "onboarding_completed saved to config",
    ))
}

/// Returns the current dictation settings as a JSON object.
pub async fn get_dictation_settings(
    ctx: &DispatchCtx,
) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
    let result = json!({
        "enabled": config.dictation.enabled,
        "hotkey": config.dictation.hotkey,
        "activation_mode": config.dictation.activation_mode,
        "llm_refinement": config.dictation.llm_refinement,
        "streaming": config.dictation.streaming,
        "streaming_interval_ms": config.dictation.streaming_interval_ms,
    });
    Ok(Outcome::new(
        result,
        vec!["dictation settings read".to_string()],
    ))
}

/// Loads configuration, applies dictation settings updates, and saves it.
pub async fn load_and_apply_dictation_settings(
    ctx: &DispatchCtx,
    update: DictationSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    if let Some(enabled) = update.enabled {
        config.dictation.enabled = enabled;
    }
    if let Some(hotkey) = update.hotkey {
        config.dictation.hotkey = hotkey;
    }
    if let Some(mode) = update.activation_mode {
        match mode.as_str() {
            "toggle" => {
                config.dictation.activation_mode = crate::config::DictationActivationMode::Toggle;
            }
            "push" => {
                config.dictation.activation_mode = crate::config::DictationActivationMode::Push;
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
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "dictation settings saved to {}",
            config.config_path.display()
        )],
    ))
}

/// Returns the current voice server settings as a JSON object.
pub async fn get_voice_server_settings(
    ctx: &DispatchCtx,
) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
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
        "stt_engine": config.voice_server.stt_engine,
    });
    Ok(Outcome::new(
        result,
        vec!["voice server settings read".to_string()],
    ))
}

/// Loads configuration, applies voice server settings updates, and saves it.
pub async fn load_and_apply_voice_server_settings(
    ctx: &DispatchCtx,
    update: VoiceServerSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    if let Some(auto_start) = update.auto_start {
        config.voice_server.auto_start = auto_start;
    }
    if let Some(hotkey) = update.hotkey {
        config.voice_server.hotkey = hotkey;
    }
    if let Some(mode) = update.activation_mode {
        match mode.as_str() {
            "tap" => {
                config.voice_server.activation_mode = crate::config::VoiceActivationMode::Tap;
            }
            "push" => {
                config.voice_server.activation_mode = crate::config::VoiceActivationMode::Push;
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
    if let Some(engine) = update.stt_engine {
        // Reject rather than silently defaulting: an unknown engine name means
        // the caller and the core disagree about what is available, and quietly
        // routing to the backend proxy would bill the wrong account.
        let parsed = crate::config::SttEngine::parse(&engine).ok_or_else(|| {
            format!("invalid stt_engine: {engine} (valid: backend, elevenlabs, openai)")
        })?;
        config.voice_server.stt_engine = parsed;
    }
    config.save().await.map_err(|e| e.to_string())?;
    let snapshot = snapshot_config_json(&config)?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "voice server settings saved to {}",
            config.config_path.display()
        )],
    ))
}
