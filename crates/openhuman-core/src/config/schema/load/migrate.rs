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
    // Embeddings have a deliberate opt-out, not a cloud-provider slug.
    // Preserve it on every load, including whitespace the settings RPC trims.
    if config
        .embeddings_provider
        .as_deref()
        .is_none_or(|provider| provider.trim() != "none")
    {
        rewrite(&mut config.embeddings_provider);
    }

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

/// Convert the single-engine `[search]` format into providers, routes and
/// roles. In-memory and idempotent like the other load migrations: the legacy
/// fields are never serialized, so the next save writes the new format.
pub(crate) fn migrate_search_settings(config: &mut Config) {
    if !config.search.needs_migration() {
        return;
    }
    let legacy = super::super::LegacySearchInputs {
        tinyfish_active: config.integrations.tinyfish.is_active(),
        seltz_active: config.seltz.enabled
            && config
                .seltz
                .api_key
                .as_deref()
                .is_some_and(|key| !key.trim().is_empty()),
        searxng_active: config.searxng.enabled,
        tinyfish_api_key: config.integrations.tinyfish.api_key.clone(),
    };
    config.search.migrate_legacy(legacy);
}

/// Migrates legacy v1 `[[memory_sources]]` into `[[memory.sources]]`, once:
/// only when no v2 source exists yet. Kinds without a v2 equivalent
/// (`twitter_query`, `conversation`, and the removed `composio`) and disabled entries are dropped. The
/// legacy list is cleared either way, so it is never written back.
pub(crate) fn migrate_legacy_memory_sources(config: &mut Config) {
    let legacy = std::mem::take(&mut config.legacy_memory_sources);
    if legacy.is_empty() || !config.memory.sources.is_empty() {
        return;
    }
    let migrated: Vec<_> = legacy
        .iter()
        .filter_map(super::super::memory::migrate_legacy_source)
        .collect();
    tracing::info!(
        legacy = legacy.len(),
        migrated = migrated.len(),
        "[config] migrated legacy memory sources"
    );
    config.memory.sources = migrated;
}

/// Disables the removed v1 memory backend and records a durable diagnostic.
pub(crate) fn migrate_legacy_memory_backend(config: &mut Config, raw: &str) {
    let Ok(value) = toml::from_str::<toml::Value>(raw) else {
        return;
    };
    let Some(memory) = value.get("memory").and_then(toml::Value::as_table) else {
        return;
    };
    let has_backend = memory
        .get("backend")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|backend| !backend.is_empty())
        .is_some();
    if !has_backend {
        return;
    }
    if memory.contains_key("engine") {
        return;
    }

    config.memory.engine.clear();
    config.memory.legacy_backend_unsupported = true;
    tracing::warn!(
        "[config] legacy memory backend is unsupported; select an explicit v2 memory engine"
    );
}
