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

use crate::openhuman::tokenjuice::types::{CompressorKind, ContentKind};

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

/// Install the attribution model and snapshot location, loading any prior
/// snapshot. Called once at startup from [`crate::openhuman::tokenjuice::install_config`].
pub fn configure(attribution_model: String, workspace_dir: &std::path::Path) {
    let path = workspace_dir.join("state").join("tokenjuice_savings.json");
    let loaded = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<SavingsAggregate>(&s).ok());
    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    if !attribution_model.trim().is_empty() {
        st.attribution_model = attribution_model;
    }
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
    let saved = original_tokens - compacted_tokens;

    let mut st = state().lock().unwrap_or_else(|p| p.into_inner());
    let model = st.attribution_model.clone();
    let cost = cost_saved_usd(&model, saved);

    st.aggregate
        .total
        .add(original_tokens, compacted_tokens, cost);
    st.aggregate
        .by_model
        .entry(model)
        .or_default()
        .add(original_tokens, compacted_tokens, cost);
    st.aggregate
        .by_compressor
        .entry(compressor.as_str().to_string())
        .or_default()
        .add(original_tokens, compacted_tokens, cost);

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
mod tests {
    use super::*;

    #[test]
    fn records_and_aggregates() {
        // Use a fresh local state to avoid clobbering the process-global one.
        let mut agg = SavingsAggregate::default();
        let cost = cost_saved_usd("agentic-v1", 1000);
        agg.total.add(2000, 1000, cost);
        agg.by_compressor
            .entry("smartcrusher".into())
            .or_default()
            .add(2000, 1000, cost);
        assert_eq!(agg.total.tokens_saved, 1000);
        assert!(agg.total.cost_saved_usd > 0.0);
        assert_eq!(agg.by_compressor["smartcrusher"].events, 1);
    }

    #[test]
    fn cost_uses_input_price() {
        // agentic-v1 input pricing is used for saved-token cost estimates.
        let c = cost_saved_usd("agentic-v1", 1_000_000);
        assert!((c - 0.435).abs() < 1e-6, "got {c}");
    }

    #[test]
    fn no_record_when_not_smaller() {
        let before = stats().total.events;
        record(ContentKind::Json, CompressorKind::SmartCrusher, 100, 100);
        record(ContentKind::Json, CompressorKind::SmartCrusher, 50, 100);
        assert_eq!(stats().total.events, before, "no-op when not smaller");
    }
}
