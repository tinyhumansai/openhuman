//! Environment-variable overlay applied on top of the persisted config.
//!
//! The entry points and the process-wide side effects live here; each
//! config section's overrides live in a submodule below.

mod components;
mod context;
mod embeddings;
mod inbound_debounce;
mod modules;
mod observability;
mod proxy;
mod runtime;
mod search;
mod update;

use super::super::proxy::{set_runtime_proxy_config, ProxyScope};
use super::super::Config;
use std::path::PathBuf;

/// Classification of an `OPENHUMAN_SHELL_HIDE_WINDOW` env value. Split out from
/// the apply site (where the three cases differ only by log level) so the
/// empty-vs-unrecognized distinction is unit-testable without capturing tracing
/// output — a bare `VAR=` must classify as `Unset` (silent no-op), not
/// `Unrecognized` (which warns).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ShellHideWindowParse {
    /// Empty / whitespace-only — the var is present but has no value; treat as
    /// absent (no change, no warning).
    Unset,
    /// A recognized boolean value.
    Set(bool),
    /// A non-empty value that isn't a recognized boolean — warn and ignore.
    Unrecognized,
}

pub(super) fn classify_shell_hide_window(raw: &str) -> ShellHideWindowParse {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" => ShellHideWindowParse::Unset,
        "1" | "true" | "yes" | "on" => ShellHideWindowParse::Set(true),
        "0" | "false" | "no" | "off" => ShellHideWindowParse::Set(false),
        _ => ShellHideWindowParse::Unrecognized,
    }
}

impl Config {
    pub(crate) fn record_env_override<E: super::env::EnvLookup + ?Sized>(
        &mut self,
        env: &E,
        field_path: &str,
        env_vars: &[&str],
    ) {
        if let Some(env_var) = env_vars.iter().copied().find(|name| env.contains(name)) {
            self.record_env_override_var(env, field_path, env_var);
        }
    }

    pub(crate) fn record_env_override_var<E: super::env::EnvLookup + ?Sized>(
        &mut self,
        env: &E,
        field_path: &str,
        env_var: &str,
    ) {
        let source = if env.is_from_dotenv(env_var) {
            super::ProvenanceSource::Dotenv
        } else {
            super::ProvenanceSource::Env
        };
        self.load_provenance_mut()
            .record_env_override(field_path, env_var, source);
    }

    pub fn apply_env_overrides(&mut self) {
        use super::env::ProcessEnv;
        self.apply_env_overrides_from(&ProcessEnv);
    }

    /// Apply ordinary config environment overrides without replacing a
    /// profile's already-selected workspace directory or updating process
    /// proxy/keyring state. Used by read-only profile inspection.
    pub(crate) fn apply_profile_env_overrides(&mut self) {
        use super::env::ProcessEnvWithoutWorkspace;
        self.apply_env_overlay_with(&ProcessEnvWithoutWorkspace);
        super::super::cli_overrides::apply_cli_inference_overrides(self);
    }

    pub(super) fn apply_env_overrides_from(
        &mut self,
        env: &(dyn super::env::EnvLookup + Send + Sync),
    ) {
        self.apply_env_overlay_with(env);

        if self.proxy.enabled && self.proxy.scope == ProxyScope::Environment {
            self.proxy.apply_to_process_env();
        }

        set_runtime_proxy_config(self.proxy.clone());

        tinyinference_embeddings::set_rate_limit(self.memory.embedding_rate_limit_per_min);

        // Launch flags are process-local and intentionally win over both the
        // persisted file and ordinary environment overlays. They are applied
        // after loading so `openhuman -p <provider> -m <model>` never mutates
        // config.toml and desktop launches remain unaffected.
        super::super::cli_overrides::apply_cli_inference_overrides(self);
    }

    /// Pure-ish env overlay: applies overrides read from `env` to `self`.
    ///
    /// "Pure-ish" because it still emits `tracing` logs and calls
    /// `self.proxy.validate()` (which only reads). Crucially, it does
    /// **not** write to the process environment nor the
    /// `set_runtime_proxy_config` global — those stay in the public
    /// [`Self::apply_env_overrides`] wrapper so unit tests can call this
    /// with a [`HashMapEnv`] (see tests) without requiring the
    /// `TEST_ENV_LOCK` or tainting sibling tests.
    pub(crate) fn apply_env_overlay_with<E: super::env::EnvLookup + ?Sized>(&mut self, env: &E) {
        self.load_provenance_mut()
            .set_dotenv(super::current_dotenv_load());

        // `OPENHUMAN_DEFAULT_MODEL` is the explicit spelling; the historical
        // `OPENHUMAN_MODEL` remains accepted. The bare `MODEL` env var used to
        // be accepted as an alias but collides with vendor
        // asset-tag env vars (e.g. Dell OptiPlex sets `MODEL=7080`), which
        // silently clobbered the LLM model and 400'd every backend call
        // (Sentry OPENHUMAN-TAURI-J8).
        let model = env
            .get("OPENHUMAN_DEFAULT_MODEL")
            .map(|value| ("OPENHUMAN_DEFAULT_MODEL", value))
            .filter(|(_, value)| !value.trim().is_empty())
            .or_else(|| {
                env.get("OPENHUMAN_MODEL")
                    .map(|value| ("OPENHUMAN_MODEL", value))
                    .filter(|(_, value)| !value.trim().is_empty())
            });
        if let Some((env_var, model)) = model {
            self.default_model = Some(model.trim().to_string());
            self.record_env_override_var(env, "default_model", env_var);
        }

        // One-launch override for the tool-call dialect experiment
        // (`agent.tool_dispatcher`): `auto | native | xml | pformat | python |
        // typescript`. Validation stays with the dispatcher mapping, which
        // falls back to `auto` with a warning on an unknown spelling.
        if let Some(raw) = env.get("OPENHUMAN_TOOL_DISPATCHER") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                tracing::debug!(
                    dispatcher = trimmed,
                    "OPENHUMAN_TOOL_DISPATCHER overrides agent.tool_dispatcher"
                );
                self.agent.tool_dispatcher = trimmed.to_string();
                self.record_env_override(
                    env,
                    "agent.tool_dispatcher",
                    &["OPENHUMAN_TOOL_DISPATCHER"],
                );
            }
        }

        // Explicit tool-iteration cap that wins over every agent definition's
        // (#6958; see `session_host::builder::iteration_cap`). A positive
        // integer; anything else is ignored with a warning.
        if let Some(raw) = env.get("OPENHUMAN_AGENT_MAX_TOOL_ITERATIONS") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                match trimmed.parse::<usize>() {
                    Ok(cap) if cap > 0 => {
                        tracing::debug!(
                            cap,
                            "OPENHUMAN_AGENT_MAX_TOOL_ITERATIONS overrides \
                             agent.max_tool_iterations_override"
                        );
                        self.agent.max_tool_iterations_override = Some(cap);
                        self.record_env_override(
                            env,
                            "agent.max_tool_iterations_override",
                            &["OPENHUMAN_AGENT_MAX_TOOL_ITERATIONS"],
                        );
                    }
                    _ => tracing::warn!(
                        value = trimmed,
                        "OPENHUMAN_AGENT_MAX_TOOL_ITERATIONS is not a positive integer; ignored"
                    ),
                }
            }
        }

        // One-launch override of `composio.mode`: `backend | direct | disabled`.
        // The factory rejects an unknown spelling loudly, so no validation here.
        if let Some(raw) = env.get("OPENHUMAN_COMPOSIO_MODE") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                tracing::debug!(
                    mode = trimmed,
                    "OPENHUMAN_COMPOSIO_MODE overrides composio.mode"
                );
                self.composio.mode = trimmed.to_ascii_lowercase();
                self.record_env_override(env, "composio.mode", &["OPENHUMAN_COMPOSIO_MODE"]);
            }
        }

        // One-launch override of how the Jev tool ranker is reached
        // (`agent.tool_search.jev_route`): `auto | tinyhumans | typesafe |
        // openrouter`. The ranker validates the spelling and falls back to
        // `auto` with a warning on an unknown one.
        if let Some(raw) = env.get("OPENHUMAN_JEV_ROUTE") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                tracing::debug!(
                    route = trimmed,
                    "OPENHUMAN_JEV_ROUTE overrides agent.tool_search.jev_route"
                );
                self.agent.tool_search.jev_route = trimmed.to_ascii_lowercase();
                self.record_env_override(
                    env,
                    "agent.tool_search.jev_route",
                    &["OPENHUMAN_JEV_ROUTE"],
                );
            }
        }
        if let Some(raw) = env.get("OPENHUMAN_JEV_BASE_URL") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                tracing::debug!("OPENHUMAN_JEV_BASE_URL overrides agent.tool_search.jev_base_url");
                self.agent.tool_search.jev_base_url = Some(trimmed.to_string());
                self.record_env_override(
                    env,
                    "agent.tool_search.jev_base_url",
                    &["OPENHUMAN_JEV_BASE_URL"],
                );
            }
        }

        if let Some(workspace) = env.get("OPENHUMAN_WORKSPACE") {
            if !workspace.is_empty() {
                let (_, workspace_dir) =
                    super::dirs::resolve_config_dir_for_workspace(&PathBuf::from(workspace));
                self.workspace_dir = workspace_dir;
                self.record_env_override(env, "workspace_dir", &["OPENHUMAN_WORKSPACE"]);
            }
        }

        if let Some(v) = env.get("OPENHUMAN_ACTION_DIR") {
            let trimmed = v.trim();
            if !trimmed.is_empty() {
                self.action_dir = PathBuf::from(trimmed);
                self.record_env_override(env, "action_dir", &["OPENHUMAN_ACTION_DIR"]);
            }
        }

        if let Some(temp_str) = env.get("OPENHUMAN_TEMPERATURE") {
            if let Ok(temp) = temp_str.parse::<f64>() {
                if (0.0..=2.0).contains(&temp) {
                    self.default_temperature = temp;
                    self.record_env_override(
                        env,
                        "default_temperature",
                        &["OPENHUMAN_TEMPERATURE"],
                    );
                }
            }
        }

        if let Some(raw) = env.get("OPENHUMAN_MAX_ACTIONS_PER_HOUR") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                match trimmed.parse::<u32>() {
                    Ok(limit) => {
                        self.autonomy.max_actions_per_hour = limit;
                        self.record_env_override(
                            env,
                            "autonomy.max_actions_per_hour",
                            &["OPENHUMAN_MAX_ACTIONS_PER_HOUR"],
                        );
                    }
                    Err(_) => tracing::warn!(
                        value = %raw,
                        "invalid OPENHUMAN_MAX_ACTIONS_PER_HOUR ignored; expected an unsigned integer"
                    ),
                }
            }
        }

        if let Some(language) = env.get("OPENHUMAN_OUTPUT_LANGUAGE") {
            let language = language.trim();
            if !language.is_empty() {
                self.output_language = Some(language.to_string());
                self.record_env_override(env, "output_language", &["OPENHUMAN_OUTPUT_LANGUAGE"]);
            }
        }

        if let Some(flag) = env.get_any(&["OPENHUMAN_REASONING_ENABLED", "REASONING_ENABLED"]) {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => {
                    self.runtime.reasoning_enabled = Some(true);
                    self.record_env_override(
                        env,
                        "runtime.reasoning_enabled",
                        &["OPENHUMAN_REASONING_ENABLED", "REASONING_ENABLED"],
                    );
                }
                "0" | "false" | "no" | "off" => {
                    self.runtime.reasoning_enabled = Some(false);
                    self.record_env_override(
                        env,
                        "runtime.reasoning_enabled",
                        &["OPENHUMAN_REASONING_ENABLED", "REASONING_ENABLED"],
                    );
                }
                _ => {}
            }
        }

        if let Some(flag) = env.get_any(&["OPENHUMAN_SHELL_HIDE_WINDOW", "SHELL_HIDE_WINDOW"]) {
            match classify_shell_hide_window(&flag) {
                // An empty / whitespace-only value means the var is present but
                // unset (common when a `.env` or launcher exports `VAR=`). Treat
                // it as absent — keep the current value rather than warning on
                // every boot. Trace-level so the no-op stays diagnosable without
                // the INFO/WARN noise this change exists to remove.
                ShellHideWindowParse::Unset => tracing::trace!(
                    "[config][shell] OPENHUMAN_SHELL_HIDE_WINDOW empty value treated as unset; \
                     keeping hide_window={}",
                    self.shell.hide_window
                ),
                ShellHideWindowParse::Set(value) => {
                    self.shell.hide_window = value;
                    self.record_env_override(
                        env,
                        "shell.hide_window",
                        &["OPENHUMAN_SHELL_HIDE_WINDOW", "SHELL_HIDE_WINDOW"],
                    );
                    tracing::debug!(
                        value = %flag,
                        "[config][shell] OPENHUMAN_SHELL_HIDE_WINDOW applied: hide_window={value}"
                    );
                }
                ShellHideWindowParse::Unrecognized => tracing::warn!(
                    value = %flag,
                    "[config][shell] OPENHUMAN_SHELL_HIDE_WINDOW unrecognized value ignored; \
                     keeping current hide_window={}",
                    self.shell.hide_window
                ),
            }
        }

        self.apply_search_env(env);
        self.apply_proxy_env(env);
        self.apply_runtime_env(env);
        self.apply_inbound_debounce_env(env);
        self.apply_observability_env(env);
        self.apply_embedding_env(env);
        self.apply_update_env(env);
        self.apply_modules_env(env);
        self.apply_components_env(env);
        self.apply_context_env(env);

        // Not an env override: the one load step every config passes through,
        // so a misspelled unattended browser action is reported, not silent.
        // Only the count: an entry is operator text and may hold anything.
        let unknown = self.browser.unknown_unattended_actions().len();
        if unknown > 0 {
            tracing::warn!(
                count = unknown,
                known = ?crate::config::schema::tools::browser::UNATTENDED_BROWSER_ACTIONS,
                "[config][browser] unattended_actions entries name no known action and are ignored"
            );
        }
    }
}
