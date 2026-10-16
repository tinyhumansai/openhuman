//! The private configuration OpenHuman hands the TinyComputer module.
//!
//! It is delivered only through the module lifecycle callback
//! (`reinitialize_module`), never logged, and rebuilt on every call so a
//! rotated or revoked credential takes effect at once:
//!
//! - `jev` — the decision model: Jev through TinyHumans when signed in, or
//!   the user's OpenRouter key; OpenJev or Sage with their own keys.
//! - `planner` — the planner, rescue and output models: through the
//!   TinyHumans proxy when signed in, else the user's OpenRouter key.
//! - `browser` — the Chrome executable the user picked, if any.
//! - `trace_path` — where desktop commands are traced, when tracing is on.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::config::{Config, DecisionModel};
use crate::inference::provider::factory::lookup_key_for_slug;
use crate::security::credentials::session_support::{
    is_local_session_token, resolve_backend_credential, BackendCredential,
};

/// A stored provider key, falling back to its environment variable.
fn provider_key(config: &Config, slug: &str, env: &str) -> Option<String> {
    lookup_key_for_slug(slug, config)
        .ok()
        .filter(|key| !key.trim().is_empty())
        .or_else(|| std::env::var(env).ok().filter(|key| !key.trim().is_empty()))
}

/// The signed-in TinyHumans bearer, if any. The offline local token is not
/// a backend credential.
fn tinyhumans_bearer(config: &Config) -> Option<String> {
    resolve_backend_credential(config)
        .ok()
        .map(BackendCredential::into_secret)
        .filter(|token| !is_local_session_token(token))
}

fn jev(config: &Config, hosted: Option<&str>) -> Option<Value> {
    match config.computer.decision_model {
        DecisionModel::Jev => match hosted {
            Some(api_key) => Some(json!({
                "api_key": api_key,
                "provider": "tiny_humans_open_router",
                "sdk_name": config.resolved_endpoints.product_identity.clone()
            })),
            // A headless or BYOK host may have no TinyHumans session. Direct
            // OpenRouter Jev remains usable with its own scoped credential.
            None => provider_key(config, "openrouter", "OPENROUTER_API_KEY")
                .map(|api_key| json!({ "api_key": api_key, "provider": "open_router" })),
        },
        DecisionModel::OpenJev => provider_key(config, "openjev", "OPENJEV_API_KEY")
            .map(|api_key| json!({ "api_key": api_key, "provider": "open_jev" })),
        DecisionModel::Sage => provider_key(config, "sage", "SAGE_API_KEY").map(|api_key| {
            json!({ "api_key": api_key, "provider": "sage", "fast": config.computer.sage_fast })
        }),
    }
}

/// The model the TinyHumans gateway runs the planner, rescue and output
/// roles on when the user picked none. The module's own defaults are
/// OpenRouter ids (`anthropic/claude-sonnet-5`, `openai/gpt-6-luna`) that the
/// gateway refuses ("Model … is not available"), so a signed-in task failed
/// before its first step; the gateway's reasoning tier can outlast its
/// request timeout on a rescue (HTTP 504). A direct OpenRouter key keeps the
/// module's defaults.
pub const HOSTED_REASONING_MODEL: &str = "agentic-v1";

/// The `planner` configuration: the hosted route with the session's bearer
/// when signed in, else the user's OpenRouter key; the user's chosen models,
/// with [`HOSTED_REASONING_MODEL`] for any unchosen role on the hosted route.
fn planner(config: &Config, hosted: Option<&str>) -> Option<Value> {
    let mut planner = match hosted {
        Some(api_key) => json!({
            "api_key": api_key,
            "provider": "tiny_humans",
            "sdk_name": config.resolved_endpoints.product_identity.clone()
        }),
        None => {
            let api_key = provider_key(config, "openrouter", "OPENROUTER_API_KEY")?;
            json!({ "api_key": api_key, "provider": "open_router" })
        }
    };
    let computer = &config.computer;
    let fallback = hosted.map(|_| HOSTED_REASONING_MODEL);
    for (key, chosen) in [
        ("model", computer.planner_model.as_deref()),
        ("rescue_model", computer.rescue_model.as_deref()),
        ("output_model", None),
    ] {
        let chosen = chosen.map(str::trim).filter(|model| !model.is_empty());
        if let Some(model) = chosen.or(fallback) {
            planner[key] = json!(model);
        }
    }
    Some(planner)
}

/// The environment switch that turns tracing on for one run.
pub const TRACE_ENV: &str = "OPENHUMAN_COMPUTER_TRACE";

/// Whether TinyComputer runs are traced: `[computer] trace`, or
/// `OPENHUMAN_COMPUTER_TRACE=1` in the environment.
#[must_use]
pub fn tracing_enabled(config: &Config) -> bool {
    tracing_enabled_with(config, std::env::var(TRACE_ENV).ok().as_deref())
}

/// [`tracing_enabled`] with the environment switch's value passed in.
fn tracing_enabled_with(config: &Config, env: Option<&str>) -> bool {
    config.computer.trace
        || matches!(
            env.map(str::trim),
            Some("1" | "true" | "TRUE" | "yes" | "YES")
        )
}

/// Where traces and task reports are written: `<workspace>/state/computer`.
#[must_use]
pub fn trace_dir(config: &Config) -> PathBuf {
    config.workspace_dir.join("state").join("computer")
}

/// Creates `dir` for traces, readable by its owner alone, also when it
/// already existed: a trace holds what the screen showed.
fn create_trace_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

/// Build the module configuration for `config`.
#[must_use]
pub fn module_config(config: &Config) -> Value {
    let hosted = tinyhumans_bearer(config);
    let mut out = Map::new();
    if let Some(jev) = jev(config, hosted.as_deref()) {
        out.insert("jev".into(), jev);
    }
    if let Some(planner) = planner(config, hosted.as_deref()) {
        out.insert("planner".into(), planner);
    }
    if let Some(executable) = config.browser.chrome_path.as_ref() {
        out.insert("browser".into(), json!({ "executable": executable }));
    }
    let traced = tracing_enabled(config);
    if traced {
        let dir = trace_dir(config);
        if let Err(error) = create_trace_dir(&dir) {
            tracing::warn!(%error, "[computer] trace directory unavailable");
        }
        out.insert(
            "trace_path".into(),
            json!(dir.join("desktop-trace.jsonl").to_string_lossy()),
        );
    }
    tracing::debug!(
        decision_model = config.computer.decision_model.as_str(),
        jev = out.contains_key("jev"),
        planner = out.contains_key("planner"),
        hosted = hosted.is_some(),
        traced,
        "[computer] module config built"
    );
    Value::Object(out)
}

/// Which account pays for TinyComputer's decisions, for the settings UI:
/// `hosted` (TinyHumans credits), `direct_openrouter` (the user's key),
/// `open_jev` or `sage` (their own keys), or `unavailable` (no credential).
#[must_use]
pub fn billing_route(config: &Config) -> &'static str {
    match module_config(config)["jev"]["provider"].as_str() {
        Some("tiny_humans_open_router") => "hosted",
        Some("open_router") => "direct_openrouter",
        Some("open_jev") => "open_jev",
        Some("sage") => "sage",
        _ => "unavailable",
    }
}

#[cfg(test)]
#[path = "computer_config_tests.rs"]
mod tests;
