//! Operator surface of the inbound event queues: depths and counts, the
//! dead-letter queue, and redriving one item; and the external-source
//! surface: offering an event, reading pinned turns' results, acknowledging
//! one.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tinybus::module::ModuleClient;
use tinyeventqueues_bus::{DeadItem, StatsResponse};

use crate::config::Config;
use crate::core::Outcome;
use crate::modules::event_queues;

use super::results::{InboundResult, ResultQuery, ResultStatus, Results};

/// The most dead letters one `inbound.dead` call returns.
pub const MAX_DEAD_LIMIT: u32 = 500;

/// The dead letters `inbound.dead` returns.
#[derive(Debug, Clone, Serialize)]
pub struct DeadLetters {
    /// Parked items, oldest first.
    pub items: Vec<DeadItem>,
}

/// The result of `inbound.redrive`.
#[derive(Debug, Clone, Serialize)]
pub struct Redriven {
    /// The redriven item's id.
    pub id: String,
    /// `false` when the item was already gone.
    pub redriven: bool,
}

fn require(config: &Config) -> Result<(), String> {
    crate::core::runtime::components::require_config(config, super::settings::COMPONENT)
}

/// Queue depths and outcome counts.
///
/// # Errors
///
/// The component is off, or the module is unavailable or failed the call.
pub async fn stats(
    config: &Config,
    client: &ModuleClient,
) -> Result<Outcome<StatsResponse>, String> {
    require(config)?;
    let stats = event_queues::stats(client)
        .await
        .map_err(|error| format!("inbound.stats: {error}"))?;
    Ok(Outcome::single_log(stats, "inbound queue stats read"))
}

/// Up to `limit` dead letters (capped at [`MAX_DEAD_LIMIT`]).
///
/// # Errors
///
/// As [`stats`].
pub async fn dead(
    config: &Config,
    client: &ModuleClient,
    limit: Option<u32>,
) -> Result<Outcome<DeadLetters>, String> {
    require(config)?;
    let limit = limit.unwrap_or(50).min(MAX_DEAD_LIMIT);
    let items = event_queues::list_dead(client, limit)
        .await
        .map_err(|error| format!("inbound.dead: {error}"))?;
    let count = items.len();
    Ok(Outcome::single_log(
        DeadLetters { items },
        format!("{count} dead letter(s)"),
    ))
}

/// Put dead letter `id` back into the pipeline.
///
/// # Errors
///
/// As [`stats`]; an empty or unknown id is refused.
pub async fn redrive(
    config: &Config,
    client: &ModuleClient,
    id: &str,
) -> Result<Outcome<Redriven>, String> {
    require(config)?;
    if id.trim().is_empty() {
        return Err("inbound.redrive: id is required".to_owned());
    }
    let redriven = event_queues::redrive(client, id)
        .await
        .map_err(|error| format!("inbound.redrive: {error}"))?;
    tracing::info!(id = %id, redriven, "[inbound] redrive");
    Ok(Outcome::single_log(
        Redriven {
            id: id.to_owned(),
            redriven,
        },
        if redriven { "redriven" } else { "already gone" },
    ))
}

/// The most results one `inbound.results` call returns.
pub const MAX_RESULTS_LIMIT: usize = 500;

/// The parameters of `inbound.offer`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferParams {
    /// A configured `[inbound.sources.*]` name.
    pub source: String,
    /// What the queues batch the source's events by.
    pub key: String,
    /// The source's own id; a redelivery is a `duplicate`.
    pub event_id: String,
    /// The event's text.
    pub text: String,
    /// The source's payload, handed to the agent as is.
    #[serde(default)]
    pub payload: Value,
    /// When the source received it; now when absent.
    #[serde(default)]
    pub received_at_ms: Option<u64>,
}

/// The result of `inbound.offer`.
#[derive(Debug, Clone, Serialize)]
pub struct Offer {
    /// What the queues did with the event: `accepted`, `duplicate`, ...
    pub disposition: String,
}

/// The parameters of `inbound.results`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultsParams {
    /// Only this source's results.
    #[serde(default)]
    pub source: Option<String>,
    /// Only results started at or after this time.
    #[serde(default)]
    pub since_ms: Option<u64>,
    /// At most this many (default 50, at most 500).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Only results with this status.
    #[serde(default)]
    pub status: Option<ResultStatus>,
}

/// The result of `inbound.results`.
#[derive(Debug, Clone, Serialize)]
pub struct ResultList {
    /// Unacknowledged results, oldest first.
    pub results: Vec<InboundResult>,
}

/// The result of `inbound.result_ack`.
#[derive(Debug, Clone, Serialize)]
pub struct Acked {
    /// Always `true`: an unknown turn is an error.
    pub acked: bool,
}

pub(crate) fn now_ms() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}

/// Offer one event of a configured external source to the queues. A
/// redelivery of an event already taken answers `duplicate`, a success.
///
/// # Errors
///
/// The component is off, the source is not configured (or unusable), a
/// field is empty, the event is too large, or the queues did not take it.
pub async fn offer(
    config: &Config,
    client: &ModuleClient,
    params: OfferParams,
) -> Result<Outcome<Offer>, String> {
    require(config)?;
    config
        .inbound
        .source(&params.source)
        .map_err(|error| format!("inbound.offer: {error}"))?;
    for (field, value) in [("key", &params.key), ("event_id", &params.event_id)] {
        if value.trim().is_empty() {
            return Err(format!("inbound.offer: {field} is required"));
        }
    }
    let event = super::events::external(
        &params.source,
        &params.key,
        &params.event_id,
        &params.text,
        params.payload,
        params.received_at_ms.unwrap_or_else(now_ms),
    )
    .map_err(|error| format!("inbound.offer: {error}"))?;
    match super::route::offer(config, client, event).await {
        super::route::Offered::Queued(disposition) => {
            let disposition = serde_json::to_value(disposition)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| format!("{disposition:?}").to_lowercase());
            tracing::debug!(
                source = %params.source,
                event_id = %params.event_id,
                disposition = %disposition,
                "[inbound] offered"
            );
            Ok(Outcome::new(Offer { disposition }, Vec::new()))
        }
        super::route::Offered::Direct(why) => Err(format!(
            "inbound.offer: the inbound queues did not take the event: {why}"
        )),
    }
}

/// Unacknowledged results of pinned turns, oldest first, after pruning those
/// past their source's `result_ttl_days`.
///
/// # Errors
///
/// The component is off, or the store failed.
pub fn results(
    config: &Config,
    store: &Results,
    params: ResultsParams,
) -> Result<Outcome<ResultList>, String> {
    require(config)?;
    let ttl_days: std::collections::HashMap<String, u32> = config
        .inbound
        .sources
        .iter()
        .map(|(name, source)| (name.clone(), source.result_ttl_days))
        .collect();
    let default_days = crate::config::schema::InboundSourceConfig::default().result_ttl_days;
    let pruned = store
        .prune(now_ms(), move |source| {
            u64::from(*ttl_days.get(source).unwrap_or(&default_days)) * 24 * 60 * 60 * 1_000
        })
        .map_err(|error| format!("inbound.results: {error:#}"))?;
    if pruned > 0 {
        tracing::debug!(pruned, "[inbound] expired results pruned");
    }
    let query = ResultQuery {
        source: params.source,
        since_ms: params.since_ms,
        status: params.status,
        limit: params.limit.unwrap_or(50).min(MAX_RESULTS_LIMIT),
    };
    let results = store
        .list(&query)
        .map_err(|error| format!("inbound.results: {error:#}"))?;
    Ok(Outcome::new(ResultList { results }, Vec::new()))
}

/// Acknowledge pinned turn `turn_id`'s result: it is no longer listed.
///
/// # Errors
///
/// The component is off, the turn is unknown, or the store failed.
pub fn result_ack(
    config: &Config,
    store: &Results,
    turn_id: &str,
) -> Result<Outcome<Acked>, String> {
    require(config)?;
    let acked = store
        .ack(turn_id)
        .map_err(|error| format!("inbound.result_ack: {error:#}"))?;
    if !acked {
        return Err(format!("inbound.result_ack: unknown turn `{turn_id}`"));
    }
    tracing::debug!(turn = %turn_id, "[inbound] result acknowledged");
    Ok(Outcome::new(Acked { acked }, Vec::new()))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
