//! `[inbound.sources.<name>]` — an external source pushed into the inbound
//! event queues over `inbound.offer` and handled by a pinned agent.
//!
//! The queues debounce a source's events per key into batches; each batch is
//! one turn of `agent_id`, with no triage, and the agent's final text is kept
//! as a result the pusher reads over `inbound.results` and acknowledges with
//! `inbound.result_ack`.
//!
//! | setting | default | meaning |
//! | --- | --- | --- |
//! | `agent_id` | required | the agent every batch runs on |
//! | `debounce_ms` | 5000 | flush a batch this long after its last event |
//! | `max_wait_ms` | 60000 | flush a batch this long after its first event at the latest |
//! | `max_batch` | 25 | the most events one turn carries |
//! | `prompt` | a short instruction | the turn's message; `{{events}}` becomes the batch as JSON |
//! | `result_ttl_days` | 7 | how long a result is kept |
//!
//! A source name is `<kind>:<name>` (`^[a-z0-9_-]+:[a-z0-9_.*-]+$`); the
//! prefixes the host's own inbound paths use (`composio:`, `webhook:`, `web:`,
//! `channel:`) are reserved.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::InboundConfig;

pub const DEFAULT_DEBOUNCE_MS: u64 = 5_000;
pub const DEFAULT_MAX_WAIT_MS: u64 = 60_000;
pub const DEFAULT_MAX_BATCH: u32 = 25;
pub const DEFAULT_RESULT_TTL_DAYS: u32 = 7;

/// Source prefixes the host's own inbound paths own.
const RESERVED_PREFIXES: [&str; 4] = ["composio:", "webhook:", "web:", "channel:"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct InboundSourceConfig {
    /// The agent every batch of this source runs on. Required.
    pub agent_id: String,
    /// Flush a batch this long after its last event, in milliseconds.
    pub debounce_ms: u64,
    /// Flush a batch this long after its first event at the latest.
    pub max_wait_ms: u64,
    /// The most events one turn carries.
    pub max_batch: u32,
    /// The turn's message; `{{events}}` is replaced with the batch as a JSON
    /// array of `{id, text, payload, received_at_ms}`. Unset, a short
    /// instruction followed by the JSON.
    pub prompt: Option<String>,
    /// How long a turn's result is kept, in days.
    pub result_ttl_days: u32,
}

impl Default for InboundSourceConfig {
    fn default() -> Self {
        Self {
            agent_id: String::new(),
            debounce_ms: DEFAULT_DEBOUNCE_MS,
            max_wait_ms: DEFAULT_MAX_WAIT_MS,
            max_batch: DEFAULT_MAX_BATCH,
            prompt: None,
            result_ttl_days: DEFAULT_RESULT_TTL_DAYS,
        }
    }
}

impl InboundSourceConfig {
    /// `Ok` when the source can run.
    ///
    /// # Errors
    ///
    /// The first unusable setting, by name.
    pub fn check(&self) -> Result<(), String> {
        if self.agent_id.trim().is_empty() {
            return Err("agent_id is required".to_owned());
        }
        if self.max_batch == 0 {
            return Err("max_batch must be at least 1".to_owned());
        }
        Ok(())
    }
}

/// `Ok` when `name` may name an external source.
///
/// # Errors
///
/// A name outside `^[a-z0-9_-]+:[a-z0-9_.*-]+$`, or under a reserved prefix.
pub fn check_source_name(name: &str) -> Result<(), String> {
    let kind_char =
        |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-');
    let well_formed = name.split_once(':').is_some_and(|(kind, rest)| {
        !kind.is_empty()
            && !rest.is_empty()
            && kind.chars().all(kind_char)
            && rest.chars().all(|c| kind_char(c) || matches!(c, '.' | '*'))
    });
    if !well_formed {
        return Err(format!(
            "source name `{name}` must match ^[a-z0-9_-]+:[a-z0-9_.*-]+$"
        ));
    }
    if let Some(prefix) = RESERVED_PREFIXES.iter().find(|p| name.starts_with(**p)) {
        return Err(format!("source prefix `{prefix}` is reserved"));
    }
    Ok(())
}

impl InboundConfig {
    /// The configuration of external source `name`.
    ///
    /// # Errors
    ///
    /// `name` is not configured, or its name or settings are unusable.
    pub fn source(&self, name: &str) -> Result<&InboundSourceConfig, String> {
        check_source_name(name)?;
        let source = self
            .sources
            .get(name)
            .ok_or_else(|| format!("inbound source `{name}` is not configured"))?;
        source
            .check()
            .map_err(|error| format!("inbound source `{name}`: {error}"))?;
        Ok(source)
    }

    /// The configured sources that can run, by name; an unusable one is
    /// logged and left out.
    pub fn usable_sources(&self) -> impl Iterator<Item = (&str, &InboundSourceConfig)> {
        self.sources
            .keys()
            .filter_map(|name| match self.source(name) {
                Ok(source) => Some((name.as_str(), source)),
                Err(error) => {
                    tracing::warn!(error = %error, "[inbound] unusable source left out");
                    None
                }
            })
    }

    /// Whether `source` is a usable external source, run on a pinned agent.
    #[must_use]
    pub fn is_pinned(&self, source: &str) -> bool {
        self.source(source).is_ok()
    }
}

#[cfg(test)]
#[path = "inbound_sources_tests.rs"]
mod tests;
