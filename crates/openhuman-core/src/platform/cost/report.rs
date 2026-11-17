//! Usage reports over the cost ledger: tokens, spend and prompt-cache hits,
//! grouped any way the caller asks.
//!
//! Pure functions of the records ([`CostTracker::records_between`]), so every
//! rule here is testable without a ledger on disk.
//!
//! - [`build_report`] groups calls by any combination of [`GroupKey`]s (day,
//!   model, provider, route, agent, thread, …) and totals tokens, cost (split
//!   into provider-charged and locally estimated) and the cache-hit ratio
//!   (`cached input ÷ input`).
//! - [`build_cache_report`] lists calls one by one with their cache hit, and
//!   counts the calls that should have hit the cache and did not: a call
//!   after the first in its thread with no cached input means the prompt's
//!   prefix changed or expired. It also prices what the uncached input cost
//!   above the cached rate, for models the catalog prices.
//!
//! [`CostTracker::records_between`]: super::tracker::CostTracker::records_between

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};

use super::route::{route_for_model, CostRoute};
use super::types::{CostRecord, CostSource};

/// A dimension a report can be grouped by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKey {
    /// Calendar day (UTC), `YYYY-MM-DD`.
    Day,
    /// ISO week, `YYYY-Www`.
    Week,
    /// Calendar month, `YYYY-MM`.
    Month,
    Model,
    Provider,
    /// `managed` or `byok`.
    Route,
    /// The agent definition that made the call.
    Agent,
    Thread,
    /// What started the turn (`web_chat`, `cron`, …).
    Origin,
    /// The embedded or SaaS user agent.
    SessionAgent,
}

impl GroupKey {
    fn name(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Model => "model",
            Self::Provider => "provider",
            Self::Route => "route",
            Self::Agent => "agent",
            Self::Thread => "thread",
            Self::Origin => "origin",
            Self::SessionAgent => "session_agent",
        }
    }

    fn value(self, record: &CostRecord) -> String {
        let usage = &record.usage;
        let ts = usage.timestamp;
        let scope = &usage.scope;
        let or_unknown = |v: &Option<String>| v.clone().unwrap_or_else(|| UNKNOWN.to_string());
        match self {
            Self::Day => ts.format("%Y-%m-%d").to_string(),
            Self::Week => {
                let week = ts.iso_week();
                format!("{}-W{:02}", week.year(), week.week())
            }
            Self::Month => ts.format("%Y-%m").to_string(),
            Self::Model => usage.model.clone(),
            Self::Provider => or_unknown(&scope.provider),
            Self::Route => route_label(&usage.model).to_string(),
            Self::Agent => or_unknown(&scope.agent_id),
            Self::Thread => or_unknown(&scope.thread_id),
            Self::Origin => or_unknown(&scope.origin),
            Self::SessionAgent => or_unknown(&scope.session_agent),
        }
    }
}

/// How far before a cache report's window its records are read, to know
/// which threads were already under way when the window opened.
pub const CACHE_LOOKBACK: chrono::Duration = chrono::Duration::hours(24);

/// The `origin` of an embedding batch's record.
pub const EMBEDDING_ORIGIN: &str = "embedding";

/// The group value of a record that does not carry that attribute (recorded
/// before attribution existed, or outside any thread).
pub const UNKNOWN: &str = "unknown";

fn route_label(model: &str) -> &'static str {
    match route_for_model(model) {
        CostRoute::Managed => "managed",
        CostRoute::Byok => "byok",
    }
}

/// Which records a report covers. Every set field must match. Unknown keys
/// are refused, so a misspelt filter cannot widen a report to everything.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportFilter {
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    /// `managed` or `byok`.
    #[serde(default)]
    pub route: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub session_agent: Option<String>,
}

impl ReportFilter {
    /// Whether `record` passes the filter.
    pub fn admits(&self, record: &CostRecord) -> bool {
        let usage = &record.usage;
        let scope = &usage.scope;
        let eq = |want: &Option<String>, have: Option<&str>| {
            want.as_deref().is_none_or(|want| have == Some(want))
        };
        eq(&self.thread_id, scope.thread_id.as_deref())
            && eq(&self.agent_id, scope.agent_id.as_deref())
            && eq(&self.model, Some(usage.model.as_str()))
            && eq(&self.provider, scope.provider.as_deref())
            && eq(&self.route, Some(route_label(&usage.model)))
            && eq(&self.origin, scope.origin.as_deref())
            && eq(&self.session_agent, scope.session_agent.as_deref())
    }
}

/// Totals for one group (or for the whole report).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ReportRow {
    /// The group's value per requested key; empty for the totals row.
    pub key: BTreeMap<String, String>,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_tokens: u64,
    pub reasoning_tokens: u64,
    pub cost_usd: f64,
    /// The part of `cost_usd` the provider reported charging.
    pub charged_usd: f64,
    /// The part of `cost_usd` estimated from the price catalog.
    pub estimated_usd: f64,
    /// Calls with neither a reported charge nor a catalogued price. Their cost
    /// is missing from `cost_usd`, so a non-zero count means it is a floor.
    pub unpriced_calls: u64,
    /// `cached_input_tokens ÷ input_tokens`; `0` with no input.
    pub cache_hit_ratio: f64,
}

impl ReportRow {
    fn add(&mut self, record: &CostRecord) {
        let usage = &record.usage;
        self.calls += 1;
        self.input_tokens += usage.input_tokens;
        self.output_tokens += usage.output_tokens;
        self.cached_input_tokens += usage.cached_input_tokens.min(usage.input_tokens);
        self.cache_write_tokens += usage.cache_creation_tokens;
        self.reasoning_tokens += usage.reasoning_tokens;
        self.cost_usd += usage.cost_usd;
        match usage.cost_source {
            CostSource::ProviderCharged => self.charged_usd += usage.cost_usd,
            CostSource::Estimated => self.estimated_usd += usage.cost_usd,
            CostSource::Unknown => self.unpriced_calls += 1,
        }
    }

    fn finish(mut self) -> Self {
        self.cache_hit_ratio = ratio(self.cached_input_tokens, self.input_tokens);
        self
    }
}

fn ratio(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

/// A grouped usage report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageReport {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub group_by: Vec<GroupKey>,
    pub totals: ReportRow,
    /// One row per group, most expensive first (then most calls, then key).
    pub rows: Vec<ReportRow>,
}

/// Group and total `records` that pass `filter`.
pub fn build_report(
    records: &[CostRecord],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    group_by: &[GroupKey],
    filter: &ReportFilter,
) -> UsageReport {
    let mut seen = HashSet::new();
    let group_by: Vec<GroupKey> = group_by
        .iter()
        .copied()
        .filter(|k| seen.insert(*k))
        .collect();
    let mut totals = ReportRow::default();
    let mut groups: HashMap<Vec<String>, ReportRow> = HashMap::new();
    for record in records.iter().filter(|r| filter.admits(r)) {
        totals.add(record);
        if group_by.is_empty() {
            continue;
        }
        let values: Vec<String> = group_by.iter().map(|k| k.value(record)).collect();
        groups
            .entry(values.clone())
            .or_insert_with(|| ReportRow {
                key: group_by
                    .iter()
                    .zip(values)
                    .map(|(k, v)| (k.name().to_string(), v))
                    .collect(),
                ..ReportRow::default()
            })
            .add(record);
    }
    let mut rows: Vec<ReportRow> = groups.into_values().map(ReportRow::finish).collect();
    rows.sort_by(|a, b| {
        b.cost_usd
            .total_cmp(&a.cost_usd)
            .then(b.calls.cmp(&a.calls))
            .then_with(|| a.key.cmp(&b.key))
    });
    UsageReport {
        from,
        to,
        group_by,
        totals: totals.finish(),
        rows,
    }
}

/// One call in a [`CacheReport`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CacheCall {
    pub timestamp: DateTime<Utc>,
    pub model: String,
    pub thread_id: Option<String>,
    pub agent_id: Option<String>,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_hit_ratio: f64,
    /// A call after the first in its thread (same model and agent) that read
    /// nothing from the cache.
    pub cold: bool,
}

/// Per-call prompt-cache behaviour.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CacheReport {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
    pub calls: Vec<CacheCall>,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_hit_ratio: f64,
    /// Calls that should have hit the cache and did not (see [`CacheCall::cold`]).
    pub cold_calls: u64,
    /// What the uncached input cost above the cached rate, for models the
    /// catalog prices — the most a perfect cache could have saved.
    pub uncached_premium_usd: f64,
}

/// The cache behaviour of the `records` in `[from, to]` that pass `filter`,
/// oldest call first. Records from before `from` are not reported; they only
/// tell which caches a call in range could already have found warm (see
/// [`CACHE_LOOKBACK`]).
pub fn build_cache_report(
    records: &[CostRecord],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    filter: &ReportFilter,
) -> CacheReport {
    // Embedding batches never read a prompt cache; they would only dilute the
    // ratio and show up as zero-hit calls.
    let mut ordered: Vec<&CostRecord> = records
        .iter()
        .filter(|r| r.usage.scope.origin.as_deref() != Some(EMBEDDING_ORIGIN))
        .filter(|r| filter.admits(r))
        .collect();
    // The id breaks timestamp ties, so the same records always classify the
    // same way.
    ordered.sort_by(|a, b| {
        a.usage
            .timestamp
            .cmp(&b.usage.timestamp)
            .then_with(|| a.id.cmp(&b.id))
    });
    // A call is a repeat only against an earlier call with the same cache
    // identity: a model, provider or agent prompt switch starts a fresh cache.
    type CacheIdentity = (String, String, Option<String>, Option<String>);
    let mut caches_seen: HashSet<CacheIdentity> = HashSet::new();
    let mut report = CacheReport {
        from,
        to,
        calls: Vec::with_capacity(ordered.len()),
        input_tokens: 0,
        cached_input_tokens: 0,
        cache_hit_ratio: 0.0,
        cold_calls: 0,
        uncached_premium_usd: 0.0,
    };
    for record in ordered {
        let usage = &record.usage;
        if usage.timestamp > to {
            continue;
        }
        let cached = usage.cached_input_tokens.min(usage.input_tokens);
        let thread = usage.scope.thread_id.clone();
        // A call from before the window only marks its cache as seen, so a
        // thread that began earlier is a repeat from its first call in range.
        if usage.timestamp < from {
            if let Some(t) = thread {
                caches_seen.insert((
                    t,
                    usage.model.clone(),
                    usage.scope.agent_id.clone(),
                    usage.scope.provider.clone(),
                ));
            }
            continue;
        }
        let repeat = thread.as_ref().is_some_and(|t| {
            !caches_seen.insert((
                t.clone(),
                usage.model.clone(),
                usage.scope.agent_id.clone(),
                usage.scope.provider.clone(),
            ))
        });
        let cold = repeat && cached == 0 && usage.input_tokens > 0;
        if cold {
            report.cold_calls += 1;
        }
        // Through the tier-aware estimator: what this call's input cost against
        // what it would have cost had every input token been cached.
        let actual = tinyagents_registry::pricing::estimate_cost_usd(
            &usage.model,
            usage.input_tokens,
            0,
            cached,
        );
        let all_cached = tinyagents_registry::pricing::estimate_cost_usd(
            &usage.model,
            usage.input_tokens,
            0,
            usage.input_tokens,
        );
        report.uncached_premium_usd += (actual - all_cached).max(0.0);
        report.input_tokens += usage.input_tokens;
        report.cached_input_tokens += cached;
        report.calls.push(CacheCall {
            timestamp: usage.timestamp,
            model: usage.model.clone(),
            thread_id: thread,
            agent_id: usage.scope.agent_id.clone(),
            input_tokens: usage.input_tokens,
            cached_input_tokens: cached,
            cache_write_tokens: usage.cache_creation_tokens,
            cache_hit_ratio: ratio(cached, usage.input_tokens),
            cold,
        });
    }
    report.cache_hit_ratio = ratio(report.cached_input_tokens, report.input_tokens);
    report
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
