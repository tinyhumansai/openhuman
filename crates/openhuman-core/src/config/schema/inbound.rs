//! `[inbound]` — inbound event queues (the `tinyeventqueues` module).
//!
//! Composio triggers and agent webhooks are ingested into the module, which
//! persists them on the host's storage, rejects redeliveries, filters and
//! paces them, and calls the host back to run triage (`Deliver`) — retrying a
//! deferred triage at the time it asked for instead of dropping it.
//!
//! The component switch is `[components] eventqueues`; `enabled` here turns
//! the routing off while leaving the component on. With either off — or with
//! the module unavailable on this host — events take the direct triage path.
//!
//! | setting | meaning |
//! | --- | --- |
//! | `enabled` | route inbound events through the queues |
//! | `queues_path` | a `queues.json` replacing the shipped one |
//! | `pipeline_path` | a `pipeline.json` replacing the shipped one |
//! | `queue_url` | where the queues live: `storage` (default) or a broker URL |
//! | `triage_timeout_ms` | how long one delivered event's triage may take (at most 25 s) |
//! | `sources.<name>` | an external source pushed over `inbound.offer`, batched and run on a pinned agent ([`InboundSourceConfig`]) |

use std::collections::BTreeMap;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use super::inbound_sources::InboundSourceConfig;

/// The longest one delivered event's triage may take. The module waits 30 s
/// for a host callback before it treats the delivery as failed and retries
/// it; a triage still running then would decide (and act) a second time on
/// the redelivery. Staying under that deadline keeps one decision per turn.
pub const MAX_TRIAGE_TIMEOUT_MS: u64 = 25_000;

/// Default bound on one delivered event's triage: [`MAX_TRIAGE_TIMEOUT_MS`].
pub const DEFAULT_TRIAGE_TIMEOUT_MS: u64 = MAX_TRIAGE_TIMEOUT_MS;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct InboundConfig {
    /// Route Composio triggers and agent webhooks through the inbound event
    /// queues. Off, they take the direct triage path.
    pub enabled: bool,
    /// A `queues.json` (a storage `QueueManifest`) replacing the shipped one.
    /// The host declares exactly these queues on the module's scope.
    pub queues_path: Option<PathBuf>,
    /// A `pipeline.json` replacing the shipped one: gate, coalescing,
    /// salience, lanes, cadences and retry tuning.
    pub pipeline_path: Option<PathBuf>,
    /// Where the queues live: unset or `storage` keeps them in the module's
    /// storage scope; `amqp://` / `amqps://` selects a `RabbitMQ` broker
    /// (only in builds with the storage `rabbitmq` driver). Never logged.
    pub queue_url: Option<String>,
    /// How long one delivered event's triage may take, in milliseconds,
    /// before the delivery is retried. Capped at [`MAX_TRIAGE_TIMEOUT_MS`];
    /// read when the host callbacks are served (the next core start).
    pub triage_timeout_ms: u64,
    /// External sources pushed over `inbound.offer`, by source name
    /// (`<kind>:<name>`, e.g. `x:mentions`): each is debounced into batches
    /// and run on its pinned agent, without triage.
    pub sources: BTreeMap<String, InboundSourceConfig>,
}

impl InboundConfig {
    /// [`Self::triage_timeout_ms`], capped below the module's callback
    /// deadline.
    #[must_use]
    pub fn triage_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.triage_timeout_ms.clamp(1, MAX_TRIAGE_TIMEOUT_MS))
    }
}

impl std::fmt::Debug for InboundConfig {
    /// Redacts credentials: a broker URL carries the broker password.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InboundConfig")
            .field("enabled", &self.enabled)
            .field("queues_path", &self.queues_path)
            .field("pipeline_path", &self.pipeline_path)
            .field(
                "queue_url",
                &self.queue_url.as_deref().map(super::storage::redact_url),
            )
            .field("triage_timeout_ms", &self.triage_timeout_ms)
            .field("sources", &self.sources)
            .finish()
    }
}

impl Default for InboundConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            queues_path: None,
            pipeline_path: None,
            queue_url: None,
            triage_timeout_ms: DEFAULT_TRIAGE_TIMEOUT_MS,
            sources: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
#[path = "inbound_tests.rs"]
mod tests;
