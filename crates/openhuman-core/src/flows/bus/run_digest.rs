//! Post-run memory digest: [`FlowRunDigestSubscriber`] stores a compact,
//! bounded summary of every successful run as a memory document tagged with
//! the flow ([`flow_tag`]) and [`FLOW_RUN_DIGEST_TAG`], so later runs can
//! recall what they already did.

use crate::config::Config;
use crate::core::events::DomainEvent;
use crate::flows::store;
use crate::flows::{flow_meta, flow_tag, FlowRun};
use crate::memory::types::{ForgetParams, ItemsListParams, MAX_LIMIT};
use crate::memory::{MemoryError, MemoryResult};
use async_trait::async_trait;
use std::sync::Arc;
use tinybus::EventHandler;
use tinymemory::{ItemKind, MetaFilter, StoreItem};

/// Bounds a post-run memory digest to a compact, LLM-cheap size — a single
/// run's summary must never dominate a later `flow_memory_recall`.
pub(super) const DIGEST_MAX_CHARS: usize = 1000;

/// Cap on how many run digests [`FlowRunDigestSubscriber`] keeps per flow
/// before forgetting the oldest.
pub(super) const DIGEST_RETENTION_CAP: usize = 50;

/// Tag carried by every post-run digest document.
pub(super) const FLOW_RUN_DIGEST_TAG: &str = "flow_run_digest";

/// The filter matching `flow_id`'s run digests: the flow's own source id
/// (stamped by [`flow_meta`]) AND the digest tag.
pub(super) fn digest_filter(flow_id: &str) -> MetaFilter {
    MetaFilter {
        kinds: vec![ItemKind::Document],
        source_id: Some(flow_tag(flow_id)),
        tags_any: vec![FLOW_RUN_DIGEST_TAG.to_string()],
        ..MetaFilter::default()
    }
}

/// Listens for `DomainEvent::FlowRunFinished` and, on a successful terminal
/// status, stores a compact digest of the run in the flow's own memory —
/// e.g. so a later run of the same scheduled digest flow can
/// `flow_memory_recall` what it already sent without re-deriving that from
/// the target service.
///
/// Success-only: `"failed"` / `"cancelled"` / `"interrupted"` / any other
/// terminal status is ignored, since a digest of a run that didn't actually
/// complete its work would misleadingly look like a record of real output.
///
/// Best-effort throughout: memory off is a quiet skip, and every other
/// failure is logged and swallowed — by the time this subscriber observes
/// `FlowRunFinished` the run has settled its own `flow_runs` row, so a
/// memory hiccup must never retroactively affect run status.
pub struct FlowRunDigestSubscriber {
    config: Arc<Config>,
}

impl FlowRunDigestSubscriber {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    async fn handle_finished(&self, flow_id: &str, run_id: &str, status: &str) {
        if status != "completed" && status != "completed_with_warnings" {
            tracing::trace!(target: "flows", %flow_id, %run_id, %status, "[flows] digest: ignoring non-success terminal status");
            return;
        }
        if !crate::memory::memory_is_on(&self.config) {
            tracing::debug!(target: "flows", %flow_id, %run_id, "[flows] digest: memory is off — skipping");
            return;
        }

        let flow_name = match store::get_flow(&self.config, flow_id) {
            Ok(Some(flow)) => flow.name,
            Ok(None) => {
                tracing::debug!(target: "flows", %flow_id, %run_id, "[flows] digest: flow no longer exists — skipping");
                return;
            }
            Err(e) => {
                tracing::warn!(target: "flows", %flow_id, %run_id, error = %e, "[flows] digest: failed to load flow — skipping");
                return;
            }
        };

        let run = match store::get_flow_run(&self.config, run_id) {
            Ok(Some(run)) => run,
            Ok(None) => {
                tracing::warn!(target: "flows", %flow_id, %run_id, "[flows] digest: run row not found — skipping");
                return;
            }
            Err(e) => {
                tracing::warn!(target: "flows", %flow_id, %run_id, error = %e, "[flows] digest: failed to load run — skipping");
                return;
            }
        };

        let digest = render_run_digest(&flow_name, &run);
        let mut meta = flow_meta(flow_id, &[FLOW_RUN_DIGEST_TAG.to_string()]);
        meta.observed_at = Some(chrono::Utc::now());
        match crate::memory::ops::store_item(&self.config, StoreItem::document(digest, meta)).await
        {
            Ok(receipt) => {
                tracing::debug!(target: "flows", %flow_id, %run_id, replayed = receipt.replayed, "[flows] digest: run digest stored");
            }
            Err(MemoryError::Off(_)) => {
                tracing::debug!(target: "flows", %flow_id, %run_id, "[flows] digest: memory is off — skipping");
                return;
            }
            Err(e) => {
                tracing::warn!(target: "flows", %flow_id, %run_id, code = e.code(), error = %e, "[flows] digest: failed to store run digest");
                return;
            }
        }

        match enforce_retention_cap(&self.config, flow_id, DIGEST_RETENTION_CAP).await {
            Ok(0) => {}
            Ok(forgotten) => {
                tracing::debug!(target: "flows", %flow_id, forgotten, "[flows] digest: retention sweep forgot stale digests");
            }
            Err(e) => {
                tracing::warn!(target: "flows", %flow_id, code = e.code(), error = %e, "[flows] digest: retention sweep failed");
            }
        }
    }
}

/// Keeps at most `cap` run digests for `flow_id`, forgetting the oldest (by
/// `observed_at`) first. Returns how many were forgotten.
pub(super) async fn enforce_retention_cap(
    config: &Config,
    flow_id: &str,
    cap: usize,
) -> MemoryResult<usize> {
    let mut digests = Vec::new();
    let mut cursor = None;
    loop {
        let page = crate::memory::ops::items_list(
            config,
            ItemsListParams {
                filter: Some(digest_filter(flow_id)),
                limit: Some(MAX_LIMIT),
                cursor,
            },
        )
        .await?;
        digests.extend(page.items);
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    if digests.len() <= cap {
        return Ok(0);
    }
    // Oldest first, so the excess taken below is the stalest digests.
    digests.sort_by_key(|digest| digest.meta.observed_at);
    let excess = digests.len() - cap;
    let ids: Vec<String> = digests
        .into_iter()
        .take(excess)
        .map(|hit| hit.id.0)
        .collect();
    let view = crate::memory::ops::forget(config, ForgetParams { ids }).await?;
    Ok(view.forgotten)
}

#[async_trait]
impl EventHandler<DomainEvent> for FlowRunDigestSubscriber {
    fn name(&self) -> &str {
        "flows::digest"
    }

    fn domains(&self) -> Option<&[&str]> {
        // `FlowRunFinished` — the only event this subscriber handles — is
        // itself tagged `"cron"` by `DomainEvent::domain()` (grouped there
        // with the other flow-run/schedule events), not `"flows"`. This is
        // matching that tag, not a typo.
        Some(&["cron"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::FlowRunFinished {
            flow_id,
            run_id,
            status,
        } = event
        {
            self.handle_finished(flow_id, run_id, status).await;
        }
    }
}

/// Truncates `s` to at most `max` `char`s, appending `…` when truncated.
pub(super) fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{truncated}…")
}

/// Composes a compact, bounded summary of a finished run: flow name,
/// finished-at, status, node count, and per-node status + truncated output.
/// Bounded to [`DIGEST_MAX_CHARS`] total.
pub(super) fn render_run_digest(flow_name: &str, run: &FlowRun) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "Flow: {flow_name}");
    let _ = writeln!(out, "Status: {}", run.status);
    if let Some(finished_at) = &run.finished_at {
        let _ = writeln!(out, "Finished: {finished_at}");
    }
    let _ = writeln!(out, "Nodes: {}", run.steps.len());
    for step in &run.steps {
        if out.chars().count() >= DIGEST_MAX_CHARS {
            break;
        }
        let status = step.status.as_deref().unwrap_or("?");
        let output = truncate_chars(&step.output.to_string(), 120);
        let _ = writeln!(out, "- {} [{status}]: {output}", step.node_id);
    }
    truncate_chars(&out, DIGEST_MAX_CHARS)
}
