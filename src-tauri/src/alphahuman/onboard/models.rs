//! Model catalog refresh and caching utilities.

use crate::openhuman::config::Config;
use crate::openhuman::providers::{canonical_china_provider_name, is_qwen_oauth_alias};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MODEL_CACHE_FILE: &str = "models_cache.json";
const MODEL_CACHE_TTL_SECS: u64 = 12 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelRefreshSource {
    Live,
    CacheFresh,
    CacheStaleFallback,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelRefreshResult {
    pub provider: String,
    pub models: Vec<String>,
    pub source: ModelRefreshSource,
    pub cache_age_secs: Option<u64>,
    pub warnings: Vec<String>,
}

impl Default for ModelRefreshSource {
    fn default() -> Self {
        Self::Live
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCacheSnapshot {
    pub provider: String,
    pub models: Vec<String>,
    pub age_secs: u64,
}

pub fn run_models_refresh(
    config: &Config,
    provider_override: Option<&str>,
    force: bool,
) -> Result<ModelRefreshResult> {
    let provider_name = provider_override
        .or(config.default_provider.as_deref())
        .unwrap_or("openrouter")
        .trim()
        .to_string();

    if provider_name.is_empty() {
        bail!("Provider name cannot be empty");
    }

    if !supports_live_model_fetch(&provider_name) {
        bail!("Provider '{provider_name}' does not support live model discovery yet");
    }

    if !force {
        if let Some(cached) = load_cached_models_for_provider(
            &config.workspace_dir,
            &provider_name,
            MODEL_CACHE_TTL_SECS,
        )? {
            return Ok(ModelRefreshResult {
                provider: provider_name,
                models: cached.models,
                source: ModelRefreshSource::CacheFresh,
                cache_age_secs: Some(cached.age_secs),
                warnings: Vec::new(),
            });
        }
    }

    let api_key = config.api_key.clone().unwrap_or_default();

    match fetch_live_models_for_provider(&provider_name, &api_key) {
        Ok(models) if !models.is_empty() => {
            cache_live_models_for_provider(&config.workspace_dir, &provider_name, &models)?;
            Ok(ModelRefreshResult {
                provider: provider_name,
                models,
                source: ModelRefreshSource::Live,
                cache_age_secs: None,
                warnings: Vec::new(),
            })
        }
        Ok(_) => {
            if let Some(stale_cache) =
                load_any_cached_models_for_provider(&config.workspace_dir, &provider_name)?
            {
                return Ok(ModelRefreshResult {
                    provider: provider_name,
                    models: stale_cache.models,
                    source: ModelRefreshSource::CacheStaleFallback,
                    cache_age_secs: Some(stale_cache.age_secs),
                    warnings: vec!["Provider returned no models; using stale cache".to_string()],
                });
            }

            bail!("Provider '{provider_name}' returned an empty model list")
        }
        Err(error) => {
            if let Some(stale_cache) =
                load_any_cached_models_for_provider(&config.workspace_dir, &provider_name)?
            {
                return Ok(ModelRefreshResult {
                    provider: provider_name,
                    models: stale_cache.models,
                    source: ModelRefreshSource::CacheStaleFallback,
                    cache_age_secs: Some(stale_cache.age_secs),
                    warnings: vec![format!("Live refresh failed: {error}")],
                });
            }

            Err(error).with_context(|| {
                format!("failed to refresh models for provider '{provider_name}'")
            })
        }
    }
}

fn canonical_provider_name(provider_name: &str) -> &str {
    if is_qwen_oauth_alias(provider_name) {
        return "qwen-code";
    }

    if let Some(canonical) = canonical_china_provider_name(provider_name) {
        return canonical;
    }

    match provider_name {
        "grok" => "xai",
        "together" => "together-ai",
        "google" | "google-gemini" => "gemini",
        "kimi_coding" | "kimi_for_coding" => "kimi-code",
        "nvidia-nim" | "build.nvidia.com" => "nvidia",
        "aws-bedrock" => "bedrock",
        _ => provider_name,
    }
}

fn allows_unauthenticated_model_fetch(provider_name: &str) -> bool {
    matches!(
        canonical_provider_name(provider_name),
        "openrouter" | "ollama" | "venice" | "astrai" | "nvidia"
    )
}

fn supports_live_model_fetch(provider_name: &str) -> bool {
    matches!(
        canonical_provider_name(provider_name),
        "openai"
            | "openrouter"
            | "anthropic"
            | "gemini"
            | "grok"
            | "xai"
            | "together-ai"
            | "nvidia"
            | "ollama"
            | "astrai"
            | "venice"
            | "qwen"
            | "qwen-code"
            | "glm"
            | "zai"
            | "bedrock"
            | "moonshot"
            | "cohere"
            | "deepseek"
            | "groq"
            | "mistral"
            | "fireworks"
    )
}

fn models_endpoint_for_provider(provider_name: &str) -> Option<&'static str> {
    match provider_name {
        "qwen-intl" => Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1/models"),
        "dashscope-us" => Some("https://dashscope-us.aliyuncs.com/compatible-mode/v1/models"),
        "moonshot-cn" | "kimi-cn" => Some("https://api.moonshot.cn/v1/models"),
        "glm-cn" | "bigmodel" => Some("https://open.bigmodel.cn/api/paas/v4/models"),
        "zai-cn" | "z.ai-cn" => Some("https://open.bigmodel.cn/api/coding/paas/v4/models"),
        _ => match canonical_provider_name(provider_name) {
            "openai" => Some("https://api.openai.com/v1/models"),
            "venice" => Some("https://api.venice.ai/api/v1/models"),
            "groq" => Some("https://api.groq.com/openai/v1/models"),
            "mistral" => Some("https://api.mistral.ai/v1/models"),
            "deepseek" => Some("https://api.deepseek.com/v1/models"),
            "xai" => Some("https://api.x.ai/v1/models"),
            "together-ai" => Some("https://api.together.xyz/v1/models"),
            "fireworks" => Some("https://api.fireworks.ai/inference/v1/models"),
            "cohere" => Some("https://api.cohere.com/compatibility/v1/models"),
            "moonshot" => Some("https://api.moonshot.ai/v1/models"),
            "glm" => Some("https://api.z.ai/api/paas/v4/models"),
            "zai" => Some("https://api.z.ai/api/coding/paas/v4/models"),
            "qwen" => Some("https://dashscope.aliyuncs.com/compatible-mode/v1/models"),
            "nvidia" => Some("https://integrate.api.nvidia.com/v1/models"),
            "astrai" => Some("https://as-trai.com/v1/models"),
            _ => None,
        },
    }
}

fn build_model_fetch_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(8))
        .connect_timeout(Duration::from_secs(4))
        .build()
        .context("failed to build model-fetch HTTP client")
}

fn normalize_model_ids(ids: Vec<String>) -> Vec<String> {
    let mut unique = BTreeSet::new();
    for id in ids {
        let trimmed = id.trim();
        if !trimmed.is_empty() {
            unique.insert(trimmed.to_string());
        }
    }
    unique.into_iter().collect()
}

fn parse_openai_compatible_model_ids(payload: &Value) -> Vec<String> {
    let mut models = Vec::new();

    if let Some(data) = payload.get("data").and_then(Value::as_array) {
        for model in data {
            if let Some(id) = model.get("id").and_then(Value::as_str) {
                models.push(id.to_string());
            }
        }
    } else if let Some(data) = payload.as_array() {
        for model in data {
            if let Some(id) = model.get("id").and_then(Value::as_str) {
                models.push(id.to_string());
            }
        }
    }

    normalize_model_ids(models)
}

fn parse_gemini_model_ids(payload: &Value) -> Vec<String> {
    let Some(models) = payload.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut ids = Vec::new();
    for model in models {
        let supports_generate_content = model
            .get("supportedGenerationMethods")
            .and_then(Value::as_array)
            .is_none_or(|methods| {
                methods
                    .iter()
                    .any(|method| method.as_str() == Some("generateContent"))
            });

        if !supports_generate_content {
            continue;
        }

        if let Some(name) = model.get("name").and_then(Value::as_str) {
            ids.push(name.trim_start_matches("models/").to_string());
        }
    }

    normalize_model_ids(ids)
}

fn parse_ollama_model_ids(payload: &Value) -> Vec<String> {
    let Some(models) = payload.get("models").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut ids = Vec::new();
    for model in models {
        if let Some(name) = model.get("name").and_then(Value::as_str) {
            ids.push(name.to_string());
        }
    }

    normalize_model_ids(ids)
}

fn fetch_openai_compatible_models(
    endpoint: &str,
    api_key: Option<&str>,
    allow_unauthenticated: bool,
) -> Result<Vec<String>> {
    let client = build_model_fetch_client()?;
    let mut request = client.get(endpoint);

    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        request = request.bearer_auth(key.trim());
    } else if !allow_unauthenticated {
        bail!("API key required for model fetch at {endpoint}");
    }

    let payload: Value = request
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .with_context(|| format!("model fetch failed: GET {endpoint}"))?
        .json()
        .context("failed to parse model list response")?;

    Ok(parse_openai_compatible_model_ids(&payload))
}

fn fetch_openrouter_models(api_key: Option<&str>) -> Result<Vec<String>> {
    let client = build_model_fetch_client()?;
    let mut request = client.get("https://openrouter.ai/api/v1/models");

    if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
        request = request.bearer_auth(key.trim());
    }

    let payload: Value = request
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .context("model fetch failed: GET https://openrouter.ai/api/v1/models")?
        .json()
        .context("failed to parse OpenRouter model list response")?;

    Ok(parse_openai_compatible_model_ids(&payload))
}

fn fetch_anthropic_models(api_key: Option<&str>) -> Result<Vec<String>> {
    let Some(api_key) = api_key else {
        bail!("Anthropic model fetch requires API key or OAuth token");
    };

    let client = build_model_fetch_client()?;
    let mut request = client
        .get("https://api.anthropic.com/v1/models")
        .header("anthropic-version", "2023-06-01");

    if api_key.starts_with("sk-ant-oat01-") {
        request = request
            .header("Authorization", format!("Bearer {api_key}"))
            .header("anthropic-beta", "oauth-2025-04-20");
    } else {
        request = request.header("x-api-key", api_key);
    }

    let response = request
        .send()
        .context("model fetch failed: GET https://api.anthropic.com/v1/models")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        bail!("Anthropic model list request failed (HTTP {status}): {body}");
    }

    let payload: Value = response
        .json()
        .context("failed to parse Anthropic model list response")?;

    Ok(parse_openai_compatible_model_ids(&payload))
}

fn fetch_gemini_models(api_key: Option<&str>) -> Result<Vec<String>> {
    let Some(api_key) = api_key else {
        bail!("Gemini model fetch requires API key");
    };

    let client = build_model_fetch_client()?;
    let payload: Value = client
        .get("https://generativelanguage.googleapis.com/v1beta/models")
        .query(&[("key", api_key), ("pageSize", "200")])
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .context("model fetch failed: GET Gemini models")?
        .json()
        .context("failed to parse Gemini model list response")?;

    Ok(parse_gemini_model_ids(&payload))
}

fn fetch_ollama_models() -> Result<Vec<String>> {
    let client = build_model_fetch_client()?;
    let payload: Value = client
        .get("http://localhost:11434/api/tags")
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .context("model fetch failed: GET http://localhost:11434/api/tags")?
        .json()
        .context("failed to parse Ollama model list response")?;

    Ok(parse_ollama_model_ids(&payload))
}

fn fetch_live_models_for_provider(provider_name: &str, api_key: &str) -> Result<Vec<String>> {
    let requested_provider_name = provider_name;
    let provider_name = canonical_provider_name(provider_name);
    let api_key = if api_key.trim().is_empty() {
        std::env::var(provider_env_var(provider_name))
            .ok()
            .or_else(|| {
                if provider_name == "anthropic" {
                    std::env::var("ANTHROPIC_OAUTH_TOKEN").ok()
                } else if provider_name == "minimax" {
                    std::env::var("MINIMAX_OAUTH_TOKEN").ok()
                } else {
                    None
                }
            })
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    } else {
        Some(api_key.trim().to_string())
    };

    let models = match provider_name {
        "openrouter" => fetch_openrouter_models(api_key.as_deref())?,
        "anthropic" => fetch_anthropic_models(api_key.as_deref())?,
        "gemini" => fetch_gemini_models(api_key.as_deref())?,
        "ollama" => {
            if api_key.as_deref().map_or(true, |k| k.trim().is_empty()) {
                fetch_ollama_models()?
            } else {
                vec![
                    "glm-5:cloud".to_string(),
                    "glm-4.7:cloud".to_string(),
                    "gpt-oss:cloud".to_string(),
                    "gemini-3-flash-preview:cloud".to_string(),
                    "qwen2.5-coder:1.5b".to_string(),
                    "qwen2.5-coder:3b".to_string(),
                    "qwen2.5:cloud".to_string(),
                    "minimax-m2.5:cloud".to_string(),
                    "deepseek-v3.1:cloud".to_string(),
                ]
            }
        }
        _ => {
            if let Some(endpoint) = models_endpoint_for_provider(requested_provider_name) {
                let allow_unauthenticated =
                    allows_unauthenticated_model_fetch(requested_provider_name);
                fetch_openai_compatible_models(endpoint, api_key.as_deref(), allow_unauthenticated)?
            } else {
                Vec::new()
            }
        }
    };

    Ok(models)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelCacheEntry {
    provider: String,
    fetched_at_unix: u64,
    models: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ModelCacheState {
    entries: Vec<ModelCacheEntry>,
}

#[derive(Debug, Clone)]
struct CachedModels {
    models: Vec<String>,
    age_secs: u64,
}

fn model_cache_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("state").join(MODEL_CACHE_FILE)
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn load_model_cache_state(workspace_dir: &Path) -> Result<ModelCacheState> {
    let path = model_cache_path(workspace_dir);
    if !path.exists() {
        return Ok(ModelCacheState::default());
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read model cache at {}", path.display()))?;

    match serde_json::from_str::<ModelCacheState>(&raw) {
        Ok(state) => Ok(state),
        Err(_) => Ok(ModelCacheState::default()),
    }
}

fn save_model_cache_state(workspace_dir: &Path, state: &ModelCacheState) -> Result<()> {
    let path = model_cache_path(workspace_dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create model cache directory {}", parent.display())
        })?;
    }

    let json = serde_json::to_vec_pretty(state).context("failed to serialize model cache")?;
    fs::write(&path, json)
        .with_context(|| format!("failed to write model cache at {}", path.display()))?;

    Ok(())
}

fn cache_live_models_for_provider(
    workspace_dir: &Path,
    provider_name: &str,
    models: &[String],
) -> Result<()> {
    let normalized_models = normalize_model_ids(models.to_vec());
    if normalized_models.is_empty() {
        return Ok(());
    }

    let mut state = load_model_cache_state(workspace_dir)?;
    let now = now_unix_secs();

    if let Some(entry) = state
        .entries
        .iter_mut()
        .find(|entry| entry.provider == provider_name)
    {
        entry.fetched_at_unix = now;
        entry.models = normalized_models;
    } else {
        state.entries.push(ModelCacheEntry {
            provider: provider_name.to_string(),
            fetched_at_unix: now,
            models: normalized_models,
        });
    }

    save_model_cache_state(workspace_dir, &state)
}

fn load_cached_models_for_provider(
    workspace_dir: &Path,
    provider_name: &str,
    ttl_secs: u64,
) -> Result<Option<CachedModels>> {
    load_cached_models_for_provider_internal(workspace_dir, provider_name, Some(ttl_secs))
}

fn load_any_cached_models_for_provider(
    workspace_dir: &Path,
    provider_name: &str,
) -> Result<Option<CachedModels>> {
    load_cached_models_for_provider_internal(workspace_dir, provider_name, None)
}

fn load_cached_models_for_provider_internal(
    workspace_dir: &Path,
    provider_name: &str,
    ttl_secs: Option<u64>,
) -> Result<Option<CachedModels>> {
    let state = load_model_cache_state(workspace_dir)?;
    let now = now_unix_secs();

    let Some(entry) = state
        .entries
        .iter()
        .find(|entry| entry.provider == provider_name)
    else {
        return Ok(None);
    };

    let age = now.saturating_sub(entry.fetched_at_unix);
    if let Some(ttl) = ttl_secs {
        if age > ttl {
            return Ok(None);
        }
    }

    Ok(Some(CachedModels {
        models: entry.models.clone(),
        age_secs: age,
    }))
}

fn provider_env_var(name: &str) -> &'static str {
    if canonical_provider_name(name) == "qwen-code" {
        return "QWEN_OAUTH_TOKEN";
    }

    match canonical_provider_name(name) {
        "openrouter" => "OPENROUTER_API_KEY",
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "ollama" => "OLLAMA_API_KEY",
        "xai" => "XAI_API_KEY",
        "together-ai" => "TOGETHER_API_KEY",
        "gemini" => "GEMINI_API_KEY",
        "qwen" => "DASHSCOPE_API_KEY",
        "glm" => "GLM_API_KEY",
        "minimax" => "MINIMAX_API_KEY",
        "kimi-code" => "KIMI_CODE_API_KEY",
        "moonshot" => "MOONSHOT_API_KEY",
        "zai" => "ZAI_API_KEY",
        "nvidia" => "NVIDIA_API_KEY",
        "astrai" => "ASTRAI_API_KEY",
        _ => "API_KEY",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_live_model_fetch_for_known_providers() {
        assert!(supports_live_model_fetch("openai"));
        assert!(supports_live_model_fetch("anthropic"));
        assert!(supports_live_model_fetch("gemini"));
        assert!(supports_live_model_fetch("grok"));
        assert!(supports_live_model_fetch("together"));
        assert!(supports_live_model_fetch("nvidia"));
        assert!(supports_live_model_fetch("ollama"));
        assert!(supports_live_model_fetch("astrai"));
        assert!(supports_live_model_fetch("venice"));
        assert!(supports_live_model_fetch("glm-cn"));
        assert!(supports_live_model_fetch("qwen-intl"));
        assert!(!supports_live_model_fetch("unknown-provider"));
    }

    #[test]
    fn allows_unauthenticated_model_fetch_for_public_catalogs() {
        assert!(allows_unauthenticated_model_fetch("openrouter"));
        assert!(allows_unauthenticated_model_fetch("venice"));
        assert!(allows_unauthenticated_model_fetch("nvidia"));
        assert!(allows_unauthenticated_model_fetch("nvidia-nim"));
        assert!(allows_unauthenticated_model_fetch("build.nvidia.com"));
        assert!(allows_unauthenticated_model_fetch("astrai"));
        assert!(allows_unauthenticated_model_fetch("ollama"));
        assert!(!allows_unauthenticated_model_fetch("openai"));
        assert!(!allows_unauthenticated_model_fetch("deepseek"));
    }

    #[test]
    fn models_endpoint_for_provider_handles_region_aliases() {
        assert_eq!(
            models_endpoint_for_provider("glm-cn"),
            Some("https://open.bigmodel.cn/api/paas/v4/models")
        );
        assert_eq!(
            models_endpoint_for_provider("zai-cn"),
            Some("https://open.bigmodel.cn/api/coding/paas/v4/models")
        );
        assert_eq!(
            models_endpoint_for_provider("qwen-intl"),
            Some("https://dashscope-intl.aliyuncs.com/compatible-mode/v1/models")
        );
    }

    #[test]
    fn provider_env_var_known_providers() {
        assert_eq!(provider_env_var("openrouter"), "OPENROUTER_API_KEY");
        assert_eq!(provider_env_var("anthropic"), "ANTHROPIC_API_KEY");
        assert_eq!(provider_env_var("openai"), "OPENAI_API_KEY");
        assert_eq!(provider_env_var("ollama"), "OLLAMA_API_KEY");
        assert_eq!(provider_env_var("xai"), "XAI_API_KEY");
        assert_eq!(provider_env_var("grok"), "XAI_API_KEY");
        assert_eq!(provider_env_var("together"), "TOGETHER_API_KEY");
        assert_eq!(provider_env_var("together-ai"), "TOGETHER_API_KEY");
        assert_eq!(provider_env_var("google"), "GEMINI_API_KEY");
        assert_eq!(provider_env_var("google-gemini"), "GEMINI_API_KEY");
        assert_eq!(provider_env_var("gemini"), "GEMINI_API_KEY");
        assert_eq!(provider_env_var("qwen"), "DASHSCOPE_API_KEY");
        assert_eq!(provider_env_var("qwen-intl"), "DASHSCOPE_API_KEY");
        assert_eq!(provider_env_var("dashscope-us"), "DASHSCOPE_API_KEY");
        assert_eq!(provider_env_var("qwen-code"), "QWEN_OAUTH_TOKEN");
        assert_eq!(provider_env_var("qwen-oauth"), "QWEN_OAUTH_TOKEN");
        assert_eq!(provider_env_var("glm-cn"), "GLM_API_KEY");
        assert_eq!(provider_env_var("minimax-cn"), "MINIMAX_API_KEY");
        assert_eq!(provider_env_var("kimi-code"), "KIMI_CODE_API_KEY");
        assert_eq!(provider_env_var("kimi_coding"), "KIMI_CODE_API_KEY");
        assert_eq!(provider_env_var("kimi_for_coding"), "KIMI_CODE_API_KEY");
        assert_eq!(provider_env_var("minimax-oauth"), "MINIMAX_API_KEY");
        assert_eq!(provider_env_var("minimax-oauth-cn"), "MINIMAX_API_KEY");
        assert_eq!(provider_env_var("moonshot-intl"), "MOONSHOT_API_KEY");
        assert_eq!(provider_env_var("zai-cn"), "ZAI_API_KEY");
        assert_eq!(provider_env_var("nvidia"), "NVIDIA_API_KEY");
        assert_eq!(provider_env_var("nvidia-nim"), "NVIDIA_API_KEY");
        assert_eq!(provider_env_var("build.nvidia.com"), "NVIDIA_API_KEY");
        assert_eq!(provider_env_var("astrai"), "ASTRAI_API_KEY");
    }

    #[test]
    fn provider_env_var_unknown_falls_back() {
        assert_eq!(provider_env_var("some-new-provider"), "API_KEY");
    }
}
