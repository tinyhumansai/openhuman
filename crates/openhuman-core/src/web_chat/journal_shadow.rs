//! Journal-shadow parity: compare the live trace spans for a turn against the
//! spans reprojected from the durable journal, and say what differs.
//!
//! Extracted from `progress_bridge` (openhuman#6419) — that file sits on a
//! grandfathered line-limit ratchet in `scripts/ci/check-openhuman-rust-layout.mjs`,
//! so this grew out of it rather than being trimmed to fit. The three functions
//! here are one unit: build a comparable signature per span, diff two signature
//! lists, and drive the comparison for one request.
//!
//! What the comparison means is documented on
//! [`journal_projection`](crate::agent::progress_tracing::journal_projection):
//! both sides fold through the same `SpanCollector`, so span-shape parity holds
//! by construction and a divergence is a real gap rather than an artefact of
//! comparing two different abstractions. The largest such gap today is that the
//! crate journals no sub-agent events at all, so every delegating turn loses
//! that subtree on replay.

fn span_projection_signature(
    spans: &[tinyagents_harness::observability::trace_export::TraceSpan],
) -> Vec<String> {
    spans
        .iter()
        .map(|span| {
            let attr_keys = span
                .attributes
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "{:?}|{}|{:?}|attrs:[{}]",
                span.kind, span.name, span.status, attr_keys
            )
        })
        .collect()
}

/// Summarise a signature mismatch as *what* differs, not how many.
///
/// The parity warning used to print two counts and two full signature vectors,
/// which for a delegating turn meant ~64 and ~21 entries of `kind|name|status`
/// on one line. That is a diff the reader has to do by eye, every turn, so the
/// warning went unactioned for long enough to be filed as noise (#6419) when the
/// signal was real: the journal carries no sub-agent events at all, so every
/// delegating turn loses that whole subtree.
///
/// Spans are matched as a multiset by signature, so a span present twice live
/// and once in the projection is reported as one missing occurrence rather than
/// as identical. Names are truncated and the list is capped — this is a log
/// line, and the point is to name the shape of the gap, not to reproduce it.
fn describe_signature_divergence(live: &[String], projected: &[String]) -> String {
    fn counts(sigs: &[String]) -> std::collections::BTreeMap<&str, i64> {
        let mut out = std::collections::BTreeMap::new();
        for sig in sigs {
            *out.entry(sig.as_str()).or_insert(0) += 1;
        }
        out
    }
    let (live_counts, projected_counts) = (counts(live), counts(projected));
    let mut missing: Vec<String> = Vec::new();
    let mut extra: Vec<String> = Vec::new();
    for (sig, live_n) in &live_counts {
        let delta = live_n - projected_counts.get(sig).copied().unwrap_or(0);
        if delta > 0 {
            missing.push(format!(
                "{delta}x {}",
                sig.chars().take(70).collect::<String>()
            ));
        }
    }
    for (sig, projected_n) in &projected_counts {
        let delta = projected_n - live_counts.get(sig).copied().unwrap_or(0);
        if delta > 0 {
            extra.push(format!(
                "{delta}x {}",
                sig.chars().take(70).collect::<String>()
            ));
        }
    }
    const MAX: usize = 6;
    let render = |label: &str, mut items: Vec<String>| -> String {
        if items.is_empty() {
            return String::new();
        }
        let total = items.len();
        items.truncate(MAX);
        let more = total.saturating_sub(MAX);
        let suffix = if more > 0 {
            format!(" (+{more} more)")
        } else {
            String::new()
        };
        format!(" {label}=[{}]{suffix}", items.join("; "))
    };
    format!(
        "{}{}",
        render("live_only", missing),
        render("journal_only", extra)
    )
}

pub(super) async fn shadow_compare_journal_projection(
    ctx: &crate::core::runtime::DispatchCtx,
    request_id: &str,
    trace_ctx: tinyagents_harness::observability::trace_export::TraceContext,
    max_iterations: u32,
    live_spans: &[tinyagents_harness::observability::trace_export::TraceSpan],
) -> Option<Vec<tinyagents_harness::observability::AgentObservation>> {
    let Some(journal_run_id) =
        crate::agent::tinyagents::journal::take_request_journal_run(ctx, request_id)
    else {
        log::debug!(
            "[agent-tracing][journal-shadow] no journal run registered request_id={}",
            request_id
        );
        return None;
    };

    let observations = match crate::agent::tinyagents::journal::read_run_events(
        ctx,
        &journal_run_id,
        0,
    )
    .await
    {
        Ok(observations) => observations,
        Err(err) => {
            log::warn!(
                "[agent-tracing][journal-shadow] read failed request_id={} journal_run_id={} err={err}",
                request_id,
                journal_run_id
            );
            return None;
        }
    };
    if observations.is_empty() {
        log::warn!(
            "[agent-tracing][journal-shadow] journal empty request_id={} journal_run_id={}",
            request_id,
            journal_run_id
        );
        return None;
    }

    let projected = crate::agent::progress_tracing::journal_projection::spans_from_observations(
        trace_ctx,
        max_iterations,
        &observations,
    );
    let live_sig = span_projection_signature(live_spans);
    let projected_sig = span_projection_signature(&projected);
    if live_sig == projected_sig {
        log::debug!(
            "[agent-tracing][journal-shadow] parity ok request_id={} journal_run_id={} spans={} observations={}",
            request_id,
            journal_run_id,
            live_spans.len(),
            observations.len()
        );
    } else {
        log::warn!(
            "[agent-tracing][journal-shadow] parity divergence request_id={} journal_run_id={} live_spans={} journal_spans={} observations={}{}",
            request_id,
            journal_run_id,
            live_spans.len(),
            projected.len(),
            observations.len(),
            describe_signature_divergence(&live_sig, &projected_sig)
        );
    }
    Some(observations)
}

#[cfg(test)]
#[path = "journal_shadow_tests.rs"]
mod tests;
