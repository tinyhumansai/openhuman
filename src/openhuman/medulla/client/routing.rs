//! Worker routing strategy and the connected-worker roster.
//!
//! Split from the client transport core; the shared request helpers
//! (`url`, `authed`, `send`) live in [`super`].

use super::*;

impl MedullaClient {
    /// Read the backend's configured worker routing strategy
    /// (`GET /medulla/v1/routing/strategy`).
    ///
    /// Returns `None` when the backend has no strategy configured or the value is
    /// unrecognized, so an absent/garbled strategy degrades to "no backend
    /// preference" rather than an error — the operator's local config still wins.
    pub async fn get_routing_strategy(&self) -> Result<Option<RoutingStrategy>> {
        let req = self.authed(self.http.get(self.url("/medulla/v1/routing/strategy")));
        let value: Value = self.send(req).await?;
        Ok(value
            .get("strategy")
            .and_then(|v| v.as_str())
            .and_then(RoutingStrategy::from_wire))
    }

    /// Persist the operator's worker routing strategy to the backend
    /// (`PUT /medulla/v1/routing/strategy`) as `{ "strategy": <camelCase> }`.
    pub async fn set_routing_strategy(&self, strategy: RoutingStrategy) -> Result<()> {
        let req = self
            .authed(self.http.put(self.url("/medulla/v1/routing/strategy")))
            .json(&serde_json::json!({ "strategy": strategy.as_wire() }));
        let _: Value = self.send(req).await?;
        Ok(())
    }

    /// Read the connected worker roster (`GET /medulla/v1/roster`).
    pub async fn roster(&self) -> Result<Vec<RosterWorker>> {
        let req = self.authed(self.http.get(self.url("/medulla/v1/roster")));
        let payload: Roster = self.send(req).await?;
        Ok(payload.workers)
    }
}

/// How the backend picks a worker for a delegated task.
///
/// The camelCase wire tokens match the backend's persisted `routingStrategy`
/// config key, so one value round-trips across config, backend, and host.
///
/// Migrated from medulla-public `runtime/types.rs`: the client is the only thing
/// that speaks these tokens over the wire, so the enum lives beside the two
/// methods that encode and decode it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RoutingStrategy {
    /// Preserve the operator's explicit host selection.
    Manual,
    /// Prefer CPU, using available memory as the tie-breaker.
    Balanced,
    /// Prefer the worker with the most logical CPU cores.
    CpuFirst,
    /// Prefer the worker with the most currently available memory.
    MemoryFirst,
}

impl RoutingStrategy {
    /// The camelCase wire token (`manual` / `balanced` / `cpuFirst` / `memoryFirst`).
    pub fn as_wire(&self) -> &'static str {
        match self {
            RoutingStrategy::Manual => "manual",
            RoutingStrategy::Balanced => "balanced",
            RoutingStrategy::CpuFirst => "cpuFirst",
            RoutingStrategy::MemoryFirst => "memoryFirst",
        }
    }

    /// Parse a camelCase wire token, or `None` when unrecognized.
    ///
    /// An unrecognized value degrades to "no backend preference" rather than an
    /// error, so a newer backend token never breaks an older host.
    pub fn from_wire(value: &str) -> Option<Self> {
        match value {
            "manual" => Some(RoutingStrategy::Manual),
            "balanced" => Some(RoutingStrategy::Balanced),
            "cpuFirst" => Some(RoutingStrategy::CpuFirst),
            "memoryFirst" => Some(RoutingStrategy::MemoryFirst),
            _ => None,
        }
    }
}

#[cfg(test)]
mod routing_strategy_tests {
    use super::RoutingStrategy;

    #[test]
    fn wire_tokens_round_trip() {
        for s in [
            RoutingStrategy::Manual,
            RoutingStrategy::Balanced,
            RoutingStrategy::CpuFirst,
            RoutingStrategy::MemoryFirst,
        ] {
            assert_eq!(RoutingStrategy::from_wire(s.as_wire()), Some(s));
        }
    }

    #[test]
    fn unknown_token_is_none_not_an_error() {
        assert_eq!(RoutingStrategy::from_wire("quantumFirst"), None);
        assert_eq!(RoutingStrategy::from_wire(""), None);
    }
}
