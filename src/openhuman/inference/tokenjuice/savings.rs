//! Compaction savings accounting — how many tokens (and $$) the content router
//! has saved.
//!
//! Every time the router compacts a tool result it records the estimated tokens
//! before/after and the cost that would have been paid to send the dropped
//! tokens as **input** to the LLM the result is being compressed for. Cost uses
//! the per-model input price from [`crate::openhuman::agent::cost`].
//!
//! Aggregates are kept process-global and snapshotted to
//! `workspace_dir/state/tokenjuice_savings.json` so the dashboard survives
//! restarts. Attribution model + snapshot path are installed once at startup
//! via [`configure`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::openhuman::inference::tokenjuice::types::{CompressorKind, ContentKind};

/// Per-key (model / compressor) rolled-up savings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavingsBucket {
    pub events: u64,
    pub original_tokens: u64,
    pub compacted_tokens: u64,
    pub tokens_saved: u64,
    pub cost_saved_usd: f64,
}

impl SavingsBucket {
    fn add(&mut self, original: u64, compacted: u64, cost: f64) {
        self.events += 1;
        self.original_tokens += original;
        self.compacted_tokens += compacted;
        self.tokens_saved += original.saturating_sub(compacted);
        self.cost_saved_usd += cost;
    }
}

/// The full savings snapshot returned to callers / the dashboard.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavingsAggregate {
    /// Overall totals across every compaction.
    pub total: SavingsBucket,
    /// Breakdown by the model the savings were attributed to.
    pub by_model: HashMap<String, SavingsBucket>,
    /// Breakdown by which compressor produced the saving.
    pub by_compressor: HashMap<String, SavingsBucket>,
}

impl SavingsAggregate {
    /// Fold one compaction's savings into the aggregate, attributed to `model`.
    /// Caller guarantees `original > compacted`. Pure (no global state) so it is
    /// unit-testable without touching the process-global aggregate.
    fn record_saving(&mut self, model: &str, compressor: &str, original: u64, compacted: u64) {
        let cost = cost_saved_usd(model, original.saturating_sub(compacted));
        self.total.add(original, compacted, cost);
        self.by_model
            .entry(model.to_string())
            .or_default()
            .add(original, compacted, cost);
        self.by_compressor
            .entry(compressor.to_string())
            .or_default()
            .add(original, compacted, cost);
    }
}

struct State {
    aggregate: SavingsAggregate,
    /// Model used to price the saved input tokens (the configured default).
    attribution_model: String,
    /// Where the snapshot is persisted; `None` ⇒ in-memory only.
    snapshot_path: Option<PathBuf>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            aggregate: SavingsAggregate::default(),
            attribution_model: crate::openhuman::config::DEFAULT_MODEL.to_string(),
            snapshot_path: None,
        }
    }
}

fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::default()))
}

tokio::task_local! {
    /// The model actually running the current turn/sub-agent, scoped around
    /// the tinyagents turn (`run_turn_via_tinyagents_shared`) — the same
    /// task-local pattern as
    /// [`crate::openhuman::agent::harness::turn_attachments_context`]. When set,
    /// compaction savings are priced against *this* model instead of the
    /// process-global configured default (issue #4122). Unset ⇒ fall back to
    /// the configured default, so non-harness callers and tests are unaffected
    /// — strictly additive.
    pub static TURN_MODEL: String;
}

/// Run `future` with `model` installed as the per-turn attribution model used
/// to price compaction savings. Intended call site is around each turn's
/// `run_turn_via_tinyagents_shared` invocation, alongside the other per-turn
/// `*_context` scopes (issue #4122).
pub async fn with_turn_model<F, R>(model: String, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    TURN_MODEL.scope(model, future).await
}

/// The model to attribute savings to: the per-turn [`TURN_MODEL`] when scoped
/// and non-empty, otherwise the process-global configured `default`.
fn resolve_attribution_model(default: &str) -> String {
    TURN_MODEL
        .try_with(|m| m.clone())
        .ok()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// Install the attribution model and snapshot location, loading a prior
/// snapshot once per workspace.
pub fn configure(attribution_model: String, workspace_dir: &std::path::Path) {
    let path = workspace_dir.join("state").join("tokenjuice_savings.json");
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    if !attribution_model.trim().is_empty() {
        st.attribution_model = attribution_model;
    }
    // Module calls apply host configuration lazily. Do not reload the snapshot
    // on every tool result: besides needless I/O, a concurrent call could have
    // read the file just before another call persisted a saving and then replace
    // the newer in-memory aggregate with that stale copy.
    if st.snapshot_path.as_ref() == Some(&path) {
        return;
    }
    let loaded = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<SavingsAggregate>(&s).ok());
    st.snapshot_path = Some(path);
    if let Some(agg) = loaded {
        st.aggregate = agg;
    }
}

/// Record one compaction's savings. `original_tokens`/`compacted_tokens` are the
/// pre/post estimates; the cost saved prices the dropped tokens as input to the
/// attribution model.
pub fn record(
    content_kind: ContentKind,
    compressor: CompressorKind,
    original_tokens: u64,
    compacted_tokens: u64,
) {
    if original_tokens <= compacted_tokens {
        return;
    }
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    // Attribute the saving to the per-turn model the agent loop scoped via
    // `with_turn_model` (issue #4122); fall back to the configured default when
    // unscoped (non-harness callers, tests).
    let model = resolve_attribution_model(&st.attribution_model);
    st.aggregate.record_saving(
        &model,
        compressor.as_str(),
        original_tokens,
        compacted_tokens,
    );

    let _ = content_kind; // reserved for a future by-kind breakdown
    persist(&st);
}

/// Cost (USD) of sending `tokens_saved` as input to `model`, using the per-model
/// input price. Tool results enter the next turn's context as input tokens, so
/// the input price is the relevant rate.
fn cost_saved_usd(model: &str, tokens_saved: u64) -> f64 {
    let pricing = crate::openhuman::agent::cost::lookup_pricing(model);
    (tokens_saved as f64) / 1_000_000.0 * pricing.input_per_mtok_usd
}

fn persist(st: &State) {
    let Some(path) = st.snapshot_path.as_ref() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string(&st.aggregate) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path, json) {
                log::debug!("[tokenjuice][savings] snapshot write failed: {e}");
            }
        }
        Err(e) => log::debug!("[tokenjuice][savings] snapshot serialize failed: {e}"),
    }
}

/// Snapshot the current savings aggregate.
pub fn stats() -> SavingsAggregate {
    state()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .aggregate
        .clone()
}

/// The model savings are currently attributed to.
pub fn attribution_model() -> String {
    state()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .attribution_model
        .clone()
}

/// Clear all recorded savings (and the persisted snapshot).
pub fn reset() {
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    st.aggregate = SavingsAggregate::default();
    persist(&st);
}

#[cfg(test)]
#[path = "savings_tests.rs"]
mod tests;
