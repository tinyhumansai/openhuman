use super::super::Config;

pub(crate) fn migrate_legacy_inference_url(config: &mut Config) {
    if config.inference_url.is_some() {
        return;
    }
    let Some(url) = config.api_url.as_deref() else {
        return;
    };
    let trimmed = url.trim().trim_end_matches('/');
    if !trimmed.ends_with("/chat/completions") {
        return;
    }
    let is_openhuman_backend = trimmed.starts_with("https://api.tinyhumans.ai/")
        || trimmed.starts_with("https://staging-api.tinyhumans.ai/");
    let moved = if is_openhuman_backend {
        None
    } else {
        Some(trimmed.to_string())
    };
    let logged = match moved.as_deref() {
        None => "<derived>".to_string(),
        Some(u) => super::redact_url_for_log(u),
    };
    tracing::info!(
        "[config][migrate] splitting legacy api_url -> inference_url (api_url cleared, inference_url={})",
        logged
    );
    config.inference_url = moved;
    config.api_url = None;
}

/// Strip userinfo (basic-auth) and query string from a URL string for log
/// emission. Falls back to a coarse `<host>/...` form when parsing fails so
/// we never leak the raw input. Public only so the migration's unit test
/// can assert the behaviour.
pub fn redact_url_for_log(raw: &str) -> String {
    if let Ok(mut url) = url::Url::parse(raw) {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
        return url.to_string();
    }
    let truncated = raw
        .split(['?', '#'])
        .next()
        .unwrap_or(raw)
        .trim_end_matches('/');
    if let Some((scheme, rest)) = truncated.split_once("://") {
        if let Some((_, host_path)) = rest.split_once('@') {
            return format!("{scheme}://***@{host_path}");
        }
        return format!("{scheme}://{rest}");
    }
    "<unparseable url>".to_string()
}

/// Migrate `cloud_providers` entries to the new slug-keyed shape and rewrite
/// any per-workload routing strings that still use the old bare-prefix grammar.
///
/// This is idempotent: entries that already have a slug/label are left
/// untouched. Routing fields that already contain a `:` are assumed to be
/// in the new `<slug>:<model>` form.
pub(crate) fn migrate_cloud_provider_slugs(config: &mut Config) {
    use super::super::cloud_providers::{migrate_legacy_fields, AuthStyle};

    for entry in &mut config.cloud_providers {
        migrate_legacy_fields(entry);
    }

    let slug_to_id: std::collections::HashMap<String, String> = config
        .cloud_providers
        .iter()
        .map(|e| (e.slug.clone(), e.id.clone()))
        .collect();

    let legacy_custom_slug = config
        .inference_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty() && !looks_like_openhuman_provider_endpoint(url))
        .and_then(|url| {
            let normalized = normalize_provider_endpoint(url);
            config
                .cloud_providers
                .iter()
                .find(|entry| {
                    !is_openhuman_provider_entry(entry)
                        && normalize_provider_endpoint(&entry.endpoint) == normalized
                })
                .map(|entry| entry.slug.clone())
        });

    let rewrite = |field: &mut Option<String>| {
        let raw = match field.as_deref() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return,
        };
        if raw.contains(':') || raw == "openhuman" {
            return;
        }
        match raw.as_str() {
            "cloud" => {
                let primary_slug = config.primary_cloud.as_deref().and_then(|pid| {
                    config
                        .cloud_providers
                        .iter()
                        .find(|e| e.id == pid)
                        .map(|e| e.slug.clone())
                });
                let slug = match primary_slug.as_deref() {
                    Some("openhuman") => legacy_custom_slug.clone().or(primary_slug),
                    Some(_) => primary_slug,
                    None => legacy_custom_slug.clone().or_else(|| {
                        config
                            .cloud_providers
                            .iter()
                            .find(|entry| !is_openhuman_provider_entry(entry))
                            .map(|entry| entry.slug.clone())
                    }),
                };
                if let Some(s) = slug {
                    if s == "openhuman" {
                        tracing::debug!(
                            "[config][migrate] rewriting routing 'cloud' → 'openhuman'"
                        );
                        *field = Some("openhuman".to_string());
                    } else {
                        tracing::info!(
                            "[config][migrate] rewriting routing 'cloud' → '{s}:' (empty model)"
                        );
                        *field = Some(format!("{s}:"));
                    }
                } else {
                    tracing::debug!(
                        "[config][migrate] routing 'cloud' with no non-openhuman provider → 'openhuman'"
                    );
                    *field = Some("openhuman".to_string());
                }
            }
            other => {
                if slug_to_id.contains_key(other) {
                    tracing::info!(
                        "[config][migrate] rewriting bare routing '{}' → '{}:'",
                        other,
                        other
                    );
                    *field = Some(format!("{other}:"));
                } else if other != "openhuman" {
                    tracing::warn!(
                        "[config][migrate] bare routing '{}' has no matching provider entry, \
                         falling back to 'openhuman'",
                        other
                    );
                    *field = Some("openhuman".to_string());
                }
            }
        }
    };

    rewrite(&mut config.reasoning_provider);
    rewrite(&mut config.agentic_provider);
    rewrite(&mut config.coding_provider);
    rewrite(&mut config.vision_provider);
    rewrite(&mut config.memory_provider);
    rewrite(&mut config.embeddings_provider);
    rewrite(&mut config.heartbeat_provider);
    rewrite(&mut config.learning_provider);
    rewrite(&mut config.subconscious_provider);

    fn normalize_provider_endpoint(url: &str) -> String {
        url.trim().trim_end_matches('/').to_ascii_lowercase()
    }

    fn looks_like_openhuman_provider_endpoint(url: &str) -> bool {
        let lower = url.trim().to_ascii_lowercase();
        let without_scheme = lower.split("://").nth(1).unwrap_or(&lower);
        let authority = without_scheme.split('/').next().unwrap_or("");
        let host = authority.split('@').next_back().unwrap_or(authority);
        let host_no_port = host.split(':').next().unwrap_or(host);
        matches!(
            host_no_port,
            "api.openhuman.ai" | "api.tinyhumans.ai" | "staging-api.tinyhumans.ai" | "openhuman"
        ) || host_no_port.ends_with(".openhuman.ai")
            || host_no_port.ends_with(".tinyhumans.ai")
    }

    fn is_openhuman_provider_entry(
        entry: &super::super::cloud_providers::CloudProviderCreds,
    ) -> bool {
        entry.slug == "openhuman"
            || matches!(entry.auth_style, AuthStyle::OpenhumanJwt)
            || looks_like_openhuman_provider_endpoint(&entry.endpoint)
    }
}

pub(super) fn migrate_legacy_autocomplete_disabled_apps(config: &mut Config) {
    let mut normalized: Vec<String> = config
        .autocomplete
        .disabled_apps
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect();
    normalized.sort();
    normalized.dedup();

    if normalized == ["code".to_string(), "terminal".to_string()] {
        config.autocomplete.disabled_apps = vec!["code".to_string()];
    }
}
