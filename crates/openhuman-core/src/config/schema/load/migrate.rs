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
    // The managed backend's own chat-completions URL is derived, not moved: it
    // is recognised by the managed hosts the installed host resolved.
    let hosts = crate::config::enrichers::managed_hosts_for(config);
    let is_openhuman_backend = trimmed.starts_with("https://")
        && crate::config::schema::resolved_endpoints::host_matches_any(trimmed, &hosts);
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
    use super::super::cloud_providers::{builtin_cloud_provider, AuthStyle};

    for entry in &mut config.cloud_providers {
        if entry.label.is_empty() {
            entry.label = builtin_cloud_provider(&entry.slug)
                .map(|provider| provider.label)
                .unwrap_or("Custom")
                .to_string();
        }
    }

    let slug_to_id: std::collections::HashMap<String, String> = config
        .cloud_providers
        .iter()
        .map(|e| (e.slug.clone(), e.id.clone()))
        .collect();

    let managed_hosts = crate::config::enrichers::managed_hosts_for(config);
    let looks_like_openhuman_provider_endpoint = |url: &str| {
        crate::config::schema::resolved_endpoints::host_matches_any(url, &managed_hosts)
    };

    let is_openhuman_provider_entry =
        |entry: &super::super::cloud_providers::CloudProviderCreds| {
            entry.slug == "openhuman"
                || matches!(entry.auth_style, AuthStyle::OpenhumanJwt)
                || looks_like_openhuman_provider_endpoint(&entry.endpoint)
        };

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

/// One-shot migration of the legacy per-dependency switches into
/// `[components]` (`hosting.enabled`, `search.enabled`, `mcp_client.enabled`,
/// `memory.engine = "none"`, `OPENHUMAN_TINYMEMES`, `composio.mode =
/// "disabled"`). `composio.enabled` was never read and is simply dropped with
/// its field; `composio.mode` stays as the route selector.
///
/// A legacy switch is read from the raw TOML (so an unset key keeps its
/// default) and only moves a component that `[components]` does not already
/// name, which makes the migration idempotent: once saved, the explicit
/// `components.*` key wins and the legacy key is never consulted again.
pub(crate) fn migrate_components(config: &mut Config, raw: &str) {
    #[cfg(feature = "tinymemes")]
    let tinymemes = std::env::var(crate::tinymemes::FLAG_ENV).ok();
    #[cfg(not(feature = "tinymemes"))]
    let tinymemes: Option<String> = None;
    migrate_components_with(config, raw, tinymemes.as_deref());
}

pub(crate) fn migrate_components_with(config: &mut Config, raw: &str, tinymemes: Option<&str>) {
    let value = toml::from_str::<toml::Value>(raw).ok();
    let explicit = |name: &str| {
        value
            .as_ref()
            .and_then(|v| v.get("components"))
            .and_then(|c| c.get(name))
            .is_some()
    };
    let legacy_bool = |table: &str| {
        value
            .as_ref()
            .and_then(|v| v.get(table))
            .and_then(|t| t.get("enabled"))
            .and_then(toml::Value::as_bool)
    };
    let mut moved: Vec<&str> = Vec::new();
    let mut disable = |config: &mut Config, name: &'static str, legacy_on: Option<bool>| {
        if legacy_on == Some(false) && !explicit(name) && config.components.get(name) == Some(true)
        {
            config.components.set(name, false);
            moved.push(name);
        }
    };
    disable(config, "hosting", legacy_bool("hosting"));
    disable(config, "search", legacy_bool("search"));
    disable(config, "mcp", legacy_bool("mcp_client"));
    let composio_off = value
        .as_ref()
        .and_then(|v| v.get("composio"))
        .and_then(|t| t.get("mode"))
        .and_then(toml::Value::as_str)
        .is_some_and(|m| m.trim().eq_ignore_ascii_case("disabled"));
    disable(config, "connectors", composio_off.then_some(false));
    let engine_none = value
        .as_ref()
        .and_then(|v| v.get("memory"))
        .and_then(|m| m.get("engine"))
        .and_then(toml::Value::as_str)
        .is_some_and(|e| e.trim().eq_ignore_ascii_case("none"));
    disable(config, "memory", engine_none.then_some(false));
    #[cfg(feature = "tinymemes")]
    let memes_on = tinymemes
        .filter(|v| !v.trim().is_empty())
        .map(crate::tinymemes::flag_enabled);
    // Without the `tinymemes` gate there is no legacy flag to migrate.
    #[cfg(not(feature = "tinymemes"))]
    let memes_on = tinymemes.and(None::<bool>);
    disable(config, "memes", memes_on);
    // The old tool-output compaction master switch, `[tokenjuice]
    // router_enabled = false`, becomes `[components] juice = false`. The
    // switch stays as a sub-setting of the on state.
    // Voice had three independent switches (`[dictation] enabled`,
    // `[voice_server] auto_start`, `always_on_enabled`). Only a config that
    // names all three and has every one off says "no voice at all"; any other
    // mix keeps the component on, so the old keys keep working for a release.
    let voice_flag = |table: &str, key: &str| {
        value
            .as_ref()
            .and_then(|v| v.get(table))
            .and_then(|t| t.get(key))
            .and_then(toml::Value::as_bool)
    };
    let voice_flags = [
        voice_flag("dictation", "enabled"),
        voice_flag("voice_server", "auto_start"),
        voice_flag("voice_server", "always_on_enabled"),
    ];
    if voice_flags.iter().all(|f| *f == Some(false)) {
        disable(config, "voice", Some(false));
    }
    let router_enabled = value
        .as_ref()
        .and_then(|v| v.get("tokenjuice"))
        .and_then(|t| t.get("router_enabled"))
        .and_then(toml::Value::as_bool);
    disable(config, "juice", router_enabled);
    // `[sandbox] enabled = false` (and the same key under `[security.sandbox]`)
    // already meant "no isolation backend"; it now also keeps the TinyBox
    // module from loading. Unset (auto) and `true` keep the component on.
    let sandbox_flag = |path: &[&str]| {
        let mut node = value.as_ref()?;
        for key in path {
            node = node.get(key)?;
        }
        node.get("enabled").and_then(toml::Value::as_bool)
    };
    let sandbox_off = [
        sandbox_flag(&["sandbox"]),
        sandbox_flag(&["security", "sandbox"]),
    ]
    .contains(&Some(false));
    disable(config, "sandbox", sandbox_off.then_some(false));
    if !moved.is_empty() {
        tracing::info!(
            "[config][migrate] legacy switches moved to [components]: {}",
            moved.join(",")
        );
    }
}
