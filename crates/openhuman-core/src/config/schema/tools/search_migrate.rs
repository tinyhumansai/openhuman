//! One-time migrations of the `[search]` section: the single-engine format to
//! providers, routes and roles (v2), then the routed role tools as the
//! presentation for files v2 had moved to `all_tools` (v3).

use std::collections::BTreeMap;

use super::{
    LegacySearchInputs, ProviderRoute, SearchConfig, SearchProviderSettings, SEARCH_ENGINE_BRAVE,
    SEARCH_ENGINE_DISABLED, SEARCH_ENGINE_EXA, SEARCH_ENGINE_MANAGED, SEARCH_ENGINE_PARALLEL,
    SEARCH_ENGINE_QUERIT, SEARCH_ENGINE_TAVILY, SEARCH_PROVIDERS, SEARCH_ROLE_SEARCH,
    SEARCH_SCHEMA_PROVIDERS, SEARCH_SCHEMA_VERSION,
};

impl SearchConfig {
    /// Whether this file was written by an older settings format.
    pub fn needs_migration(&self) -> bool {
        self.schema_version < SEARCH_SCHEMA_VERSION
    }

    /// Bring an older `[search]` section up to [`SEARCH_SCHEMA_VERSION`].
    /// Idempotent: returns `false` and changes nothing on a current file.
    pub fn migrate_legacy(&mut self, legacy: LegacySearchInputs) -> bool {
        if !self.needs_migration() {
            return false;
        }
        let legacy_tinyfish_key = legacy.tinyfish_api_key.clone();
        if self.schema_version < SEARCH_SCHEMA_PROVIDERS {
            self.migrate_single_engine(legacy);
        }
        self.migrate_presentation_to_roles();
        self.migrate_tinyfish_to_own_key(legacy_tinyfish_key);
        self.schema_version = SEARCH_SCHEMA_VERSION;
        true
    }

    /// v2 → v3: TinyFish is own-key only (the TinyHumans backend never
    /// proxied it, so every managed TinyFish call failed). Seed its key from
    /// the legacy integration toggle when there is one, and turn it off when
    /// there is no key, so it waits in "With your own key" instead of sitting
    /// in the connected list as "Needs API key".
    fn migrate_tinyfish_to_own_key(&mut self, legacy_key: Option<String>) {
        if !self.tinyfish.has_key() {
            if let Some(key) = legacy_key.filter(|k| !k.trim().is_empty()) {
                self.tinyfish.api_key = Some(key);
            }
        }
        let has_key = self.tinyfish.has_key();
        if let Some(settings) = self.providers.get_mut("tinyfish") {
            settings.route = ProviderRoute::Direct;
            if settings.enabled && !has_key {
                tracing::info!(
                    "[config][migrate][search] TinyFish needs your own key now; turned it off"
                );
                settings.enabled = false;
            }
        }
    }

    /// v2 → v3: put `all_tools` files back on the routed role tools.
    ///
    /// Agent tool scopes (orchestrator, planner, …) allowlist
    /// `web_search_tool` / `web_answer_tool` / `web_contents_tool`. Under
    /// `all_tools` TinySearch advertises only provider tools (`exa_search`,
    /// …), so those agents had no web search at all and called the missing
    /// routed names until the turn aborted. Router and one-provider choices
    /// were never forced by a migration, so they are left alone.
    fn migrate_presentation_to_roles(&mut self) {
        if self.presentation == super::PresentationMode::AllTools {
            tracing::info!(
                "[config][migrate][search] presentation all_tools -> roles (routed web tools)"
            );
            self.presentation = super::PresentationMode::Roles;
        }
    }

    /// v0/1 → v2: convert the single-engine format into providers, routes and
    /// roles.
    ///
    /// Managed selections map to managed Exa (search, contents) plus managed
    /// Gemini (answer) regardless of whether a session exists right now; the
    /// settings RPC reports them as "sign in required" until one does.
    /// Parallel stays as a bring-your-own-key provider. Only managed
    /// (backend-routed) Parallel is gone: a selection that relied on it
    /// without a key is dropped, and Exa and Gemini cover those roles.
    fn migrate_single_engine(&mut self, legacy: LegacySearchInputs) {
        let engine = self
            .engine
            .as_deref()
            .map(|e| e.trim().to_ascii_lowercase())
            .unwrap_or_else(|| SEARCH_ENGINE_MANAGED.to_string());
        let enabled = self.enabled.unwrap_or(engine != SEARCH_ENGINE_DISABLED);
        let gemini_route = self
            .gemini_route
            .as_deref()
            .and_then(ProviderRoute::parse)
            .unwrap_or(ProviderRoute::Backend);
        let mut providers = BTreeMap::new();
        let parallel_key = self.parallel.has_key();
        let managed_parallel = self
            .parallel_route
            .as_deref()
            .and_then(ProviderRoute::parse)
            == Some(ProviderRoute::Backend);
        let mut dropped_parallel = false;

        match self.enabled_providers.take() {
            Some(selected) => {
                let selected_exa_with_key = selected.contains("exa") && self.exa.has_key();
                for name in selected {
                    match name.as_str() {
                        "managed" => {
                            providers.insert(
                                "exa".into(),
                                if selected_exa_with_key {
                                    SearchProviderSettings::direct()
                                } else {
                                    SearchProviderSettings::managed()
                                },
                            );
                            providers
                                .entry("gemini".into())
                                .or_insert_with(SearchProviderSettings::managed);
                        }
                        "parallel" if parallel_key || !managed_parallel => {
                            providers.insert("parallel".into(), SearchProviderSettings::direct());
                        }
                        "parallel" => dropped_parallel = true,
                        "gemini" => {
                            providers.insert(
                                "gemini".into(),
                                SearchProviderSettings {
                                    enabled: true,
                                    route: gemini_route,
                                },
                            );
                        }
                        "tinyfish" => {
                            providers.insert("tinyfish".into(), SearchProviderSettings::managed());
                        }
                        other if SEARCH_PROVIDERS.contains(&other) => {
                            providers
                                .entry(other.to_string())
                                .or_insert_with(SearchProviderSettings::direct);
                        }
                        other => {
                            tracing::warn!(
                                provider = other,
                                "[config][migrate][search] dropping unknown provider"
                            );
                        }
                    }
                }
            }
            None => {
                let exa_route = if engine == SEARCH_ENGINE_EXA && self.exa.has_key() {
                    ProviderRoute::Direct
                } else {
                    ProviderRoute::Backend
                };
                providers.insert(
                    "exa".into(),
                    SearchProviderSettings {
                        enabled: true,
                        route: exa_route,
                    },
                );
                let gemini_route = if self.gemini.has_key() && self.gemini_route.is_some() {
                    gemini_route
                } else {
                    ProviderRoute::Backend
                };
                providers.insert(
                    "gemini".into(),
                    SearchProviderSettings {
                        enabled: true,
                        route: gemini_route,
                    },
                );
                for (name, credentials) in [
                    ("brave", &self.brave),
                    ("querit", &self.querit),
                    ("tavily", &self.tavily),
                    ("parallel", &self.parallel),
                ] {
                    if credentials.has_key() {
                        providers.insert(name.into(), SearchProviderSettings::direct());
                    }
                }
                if legacy.tinyfish_active {
                    providers.insert("tinyfish".into(), SearchProviderSettings::managed());
                }
                if legacy.seltz_active {
                    providers.insert("seltz".into(), SearchProviderSettings::direct());
                }
                if legacy.searxng_active {
                    providers.insert("searxng".into(), SearchProviderSettings::direct());
                }
            }
        }

        // These legacy toggles were independent of enabled_providers.
        if legacy.searxng_active {
            providers.insert("searxng".into(), SearchProviderSettings::direct());
        }
        if legacy.seltz_active {
            providers.insert("seltz".into(), SearchProviderSettings::direct());
        }
        if legacy.tinyfish_active {
            providers.insert("tinyfish".into(), SearchProviderSettings::managed());
        }

        // A deliberately chosen BYO engine stays first for ranked search.
        let mut roles = BTreeMap::new();
        if matches!(
            engine.as_str(),
            SEARCH_ENGINE_BRAVE
                | SEARCH_ENGINE_QUERIT
                | SEARCH_ENGINE_TAVILY
                | SEARCH_ENGINE_PARALLEL
        ) && providers.contains_key(engine.as_str())
        {
            roles.insert(
                SEARCH_ROLE_SEARCH.to_string(),
                vec![engine.clone(), "exa".into()],
            );
        }

        if dropped_parallel {
            tracing::warn!(
                "[config][migrate][search] managed Parallel is no longer offered and no \
                 Parallel key is stored; dropped it (managed Exa and Gemini cover its roles, \
                 or add your own Parallel key)"
            );
        }
        tracing::info!(
            engine = %engine,
            enabled,
            providers = ?providers.keys().collect::<Vec<_>>(),
            "[config][migrate][search] migrated single-engine search settings"
        );

        self.enabled = Some(enabled);
        self.providers = providers;
        self.roles = roles;
        if self.presentation_provider.as_deref() == Some("managed")
            || (self.presentation_provider.as_deref() == Some("parallel")
                && !self.providers.contains_key("parallel"))
        {
            self.presentation_provider = None;
        }
        // A legacy file that omitted `presentation` gets the routed role
        // tools like a fresh one; an explicit legacy choice is kept (and
        // `all_tools` then goes through the v3 step).
        self.engine = None;
        self.parallel_route = None;
        self.gemini_route = None;
        self.schema_version = SEARCH_SCHEMA_PROVIDERS;
    }

    /// Apply a legacy single-engine selection (`SEARCH_ENGINE`, or an older
    /// client sending `engine`) on top of the current provider settings.
    pub fn apply_legacy_engine(&mut self, engine: &str) -> Result<(), String> {
        let engine = engine.trim().to_ascii_lowercase();
        match engine.as_str() {
            SEARCH_ENGINE_DISABLED => {
                self.enabled = Some(false);
            }
            SEARCH_ENGINE_MANAGED => {
                self.enabled = Some(true);
                self.providers
                    .insert("exa".into(), SearchProviderSettings::managed());
                self.providers
                    .entry("gemini".into())
                    .or_insert_with(SearchProviderSettings::managed);
                self.roles.remove(SEARCH_ROLE_SEARCH);
            }
            SEARCH_ENGINE_BRAVE
            | SEARCH_ENGINE_QUERIT
            | SEARCH_ENGINE_TAVILY
            | SEARCH_ENGINE_PARALLEL
            | SEARCH_ENGINE_EXA => {
                self.enabled = Some(true);
                self.providers
                    .insert(engine.clone(), SearchProviderSettings::direct());
                let mut order = vec![engine.clone()];
                if engine != SEARCH_ENGINE_EXA {
                    order.push("exa".into());
                }
                self.roles.insert(SEARCH_ROLE_SEARCH.to_string(), order);
            }
            other => {
                return Err(format!(
                    "unknown search engine '{other}' (expected disabled, managed, brave, querit, exa, tavily or parallel)"
                ));
            }
        }
        tracing::debug!(engine = %engine, "[config][search] applied legacy engine selection");
        Ok(())
    }
}
