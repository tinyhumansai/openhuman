//! Config semantic validation: [`check_config_semantics`] and its embedding
//! provider URL check.

use crate::config::Config;

use super::types::DiagnosticItem;

pub(super) fn check_config_semantics(config: &Config, items: &mut Vec<DiagnosticItem>) {
    let cat = "config";

    // Config file exists
    if config.config_path.exists() {
        items.push(DiagnosticItem::ok(
            cat,
            format!("config file: {}", config.config_path.display()),
        ));
    } else {
        items.push(DiagnosticItem::error(
            cat,
            format!("config file not found: {}", config.config_path.display()),
        ));
    }

    // Backend API URL
    if let Some(url) = config
        .api_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        items.push(DiagnosticItem::ok(cat, format!("api_url: {url}")));
    } else {
        let resolved = config
            .resolved_endpoints
            .inference_or_configured(&config.api_url)
            .unwrap_or_else(|| "no hosted backend".to_string());
        items.push(DiagnosticItem::ok(
            cat,
            format!("api_url: (unset) resolved to {resolved}"),
        ));
    }

    match crate::security::credentials::jwt::get_session_token(config) {
        Ok(Some(token)) if !token.trim().is_empty() => {
            items.push(DiagnosticItem::ok(cat, "signed in with app session JWT"));
        }
        Ok(_) => {
            items.push(DiagnosticItem::warn(
                cat,
                "no app session JWT — not signed in",
            ));
        }
        Err(err) => {
            items.push(DiagnosticItem::error(
                cat,
                format!("failed to read app session JWT: {err}"),
            ));
        }
    }

    // Model configured
    if config.default_model.is_some() {
        items.push(DiagnosticItem::ok(
            cat,
            format!(
                "default model: {}",
                config.default_model.as_deref().unwrap_or("?")
            ),
        ));
    } else {
        items.push(DiagnosticItem::warn(cat, "no default_model configured"));
    }

    // Temperature range
    if config.default_temperature >= 0.0 && config.default_temperature <= 2.0 {
        items.push(DiagnosticItem::ok(
            cat,
            format!(
                "temperature {:.1} (valid range 0.0-2.0)",
                config.default_temperature
            ),
        ));
    } else {
        items.push(DiagnosticItem::error(
            cat,
            format!(
                "temperature {:.1} is out of range (expected 0.0-2.0)",
                config.default_temperature
            ),
        ));
    }

    // Reliability: fallback providers (legacy; ignored at runtime)
    if !config.reliability.fallback_providers.is_empty() {
        items.push(DiagnosticItem::warn(
            cat,
            "reliability.fallback_providers is set but ignored (single backend)",
        ));
    }

    // Model routes validation
    for route in &config.model_routes {
        if route.hint.is_empty() {
            items.push(DiagnosticItem::warn(cat, "model route with empty hint"));
        }
        if route.model.is_empty() {
            items.push(DiagnosticItem::warn(
                cat,
                format!("model route \"{}\" has empty model", route.hint),
            ));
        }
    }

    // Embedding routes validation
    for route in &config.embedding_routes {
        if route.hint.trim().is_empty() {
            items.push(DiagnosticItem::warn(cat, "embedding route with empty hint"));
        }
        if let Some(reason) = embedding_provider_validation_error(&route.provider) {
            items.push(DiagnosticItem::warn(
                cat,
                format!(
                    "embedding route \"{}\" uses invalid provider \"{}\": {}",
                    route.hint, route.provider, reason
                ),
            ));
        }
        if route.model.trim().is_empty() {
            items.push(DiagnosticItem::warn(
                cat,
                format!("embedding route \"{}\" has empty model", route.hint),
            ));
        }
        if route.dimensions.is_some_and(|value| value == 0) {
            items.push(DiagnosticItem::warn(
                cat,
                format!(
                    "embedding route \"{}\" has invalid dimensions=0",
                    route.hint
                ),
            ));
        }
    }

    if let Some(hint) = config
        .memory
        .embedding_model
        .strip_prefix("hint:")
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if !config
            .embedding_routes
            .iter()
            .any(|route| route.hint.trim() == hint)
        {
            items.push(DiagnosticItem::warn(
                cat,
                format!(
                    "memory.embedding_model uses hint \"{hint}\" but no matching [[embedding_routes]] entry exists"
                ),
            ));
        }
    }

    // Channel: at least one configured
    let cc = &config.channels_config;
    let has_channel = cc.telegram.is_some()
        || cc.discord.is_some()
        || cc.slack.is_some()
        || cc.imessage.is_some()
        || cc.matrix.is_some()
        || cc.whatsapp.is_some()
        || cc.email.is_some()
        || cc.irc.is_some()
        || cc.lark.is_some()
        || cc.webhook.is_some();

    if has_channel {
        items.push(DiagnosticItem::ok(cat, "at least one channel configured"));
    } else {
        items.push(DiagnosticItem::warn(
            cat,
            "no channels configured - configure one in the UI",
        ));
    }

    // Delegate agents
    let mut agent_names: Vec<_> = config.agents.keys().collect();
    agent_names.sort();
    for name in agent_names {
        let agent = config.agents.get(name).unwrap();
        if agent.model.trim().is_empty() {
            items.push(DiagnosticItem::warn(
                cat,
                format!("delegate agent \"{name}\" has empty model"),
            ));
        }
    }
}

pub(super) fn embedding_provider_validation_error(name: &str) -> Option<String> {
    let normalized = name.trim();
    if normalized.eq_ignore_ascii_case("none") || normalized.eq_ignore_ascii_case("openai") {
        return None;
    }

    let Some(url) = normalized.strip_prefix("custom:") else {
        return Some("supported values: none, openai, custom:<url>".into());
    };

    let url = url.trim();
    if url.is_empty() {
        return Some("custom provider requires a non-empty URL after 'custom:'".into());
    }

    match reqwest::Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => None,
        Ok(parsed) => Some(format!(
            "custom provider URL must use http/https, got '{}'",
            parsed.scheme()
        )),
        Err(err) => Some(format!("invalid custom provider URL: {err}")),
    }
}
