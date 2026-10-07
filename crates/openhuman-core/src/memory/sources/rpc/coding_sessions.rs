//! Coding-session discovery and ingestion handlers.

use crate::config::rpc as config_rpc;
use crate::config::Config;
use crate::core::Outcome;
use crate::memory::api::provider::sessions::{CodingSessionIngestReport, CodingSessionSource};
use crate::memory::binding::MemoryBinding;

/// The coding-session ingest request, under this domain's own name.
///
/// Re-exported here so `schemas.rs` can name it as
/// `rpc::CodingSessionIngestRequest`, the way every other handler adapter in
/// that file names its request type. The path behind the alias is now the
/// contract's (`tinymemory_api::provider::sessions`) rather than the engine's,
/// which is what lets `ingest_coding_sessions_rpc` hand the value straight to
/// the driver with no conversion in between — the request that crosses the bus
/// is the request `schemas.rs` deserialised.
pub use crate::memory::api::provider::sessions::CodingSessionIngestRequest;

/// The refusal a handler returns when the bound driver serves no such family.
///
/// One place, so the message and the log line cannot drift between the six
/// call sites, and so the driver id is always in both. See the parent module
/// docs for why these handlers refuse rather than degrade to an empty answer.
///
/// `pub(super)` because every handler cluster in `rpc` needs it, not just this
/// one — coding sessions, source sync, cost reporting, and the apply-all sweep
/// all refuse the same way.
pub(super) fn unserved(binding: &MemoryBinding, family: &str, call: &str) -> String {
    tracing::warn!(
        driver = %binding.driver_id(),
        family = %family,
        call = %call,
        "[memory_sources] refusing: bound driver does not serve this capability family"
    );
    format!(
        "the bound memory driver '{}' does not serve {family}",
        binding.driver_id()
    )
}

#[derive(Debug, serde::Serialize)]
pub struct CodingSessionStatusResponse {
    pub sources: Vec<CodingSessionSource>,
}

/// Discover what each supported coding agent's session store holds.
///
/// The scan happens driver-side and is bounded by the driver's own caps, which
/// is why this no longer needs a blocking worker: the walk that used to run on
/// this process's pool now runs behind the bus, and what comes back is counts.
/// `CodingSessionSource::scan_truncated` is how a caller learns the counts are
/// a floor — the same field the engine's `CodingSessionSourceStatus` carried,
/// under the same name.
pub async fn coding_session_status_rpc() -> Result<Outcome<CodingSessionStatusResponse>, String> {
    tracing::debug!("[memory_sources] coding_session_status_rpc: entry");
    let config = config_rpc::load_config_with_timeout().await?;
    let binding = crate::memory::binding::for_config(&config)?;
    let Some(sessions) = binding.provider().as_coding_sessions() else {
        return Err(unserved(
            &binding,
            "coding sessions",
            "coding_session_status",
        ));
    };

    let sources = sessions
        .coding_session_status()
        .await
        .map_err(|error| format!("coding session status: {error}"))?;

    tracing::debug!(
        driver = %binding.driver_id(),
        sources = sources.len(),
        files = sources
            .iter()
            .map(|source| source.session_files)
            .sum::<usize>(),
        truncated = sources.iter().any(|source| source.scan_truncated),
        "[memory_sources] coding_session_status_rpc: exit"
    );
    Ok(Outcome::new(
        CodingSessionStatusResponse { sources },
        vec![],
    ))
}

/// Wall-clock ceiling for one `ingest_coding_sessions` RPC, sized to the number
/// of sessions the caller asked to backfill and hard-capped at the ceiling the
/// frontend can actually wait for.
///
/// The original formula (`120 + N*30`) assumed **one LLM call per session**.
/// That premise is false: TinyCortex's persona pipeline splits an oversized
/// session into windows (`WINDOW_CHARS`-sized chunks of evidence) and issues one
/// LLM call *per window*, so a multi-window session drives several sequential
/// calls. A dense backfill therefore blew the old budget — 15 sessions hit the
/// exact 570 s ceiling (`120 + 15*30`) and were killed mid-flight.
///
/// The per-session allowance is therefore sized for *multiple* windows, not one
/// call: `PER_SESSION_SECS` budgets ~3 sequential per-window LLM calls at the
/// windows' observed 20–45 s span (#5509). It is a deliberate flat estimate, not
/// a per-session window count.
///
/// The result is hard-capped at `HARD_CAP_SECS` for two reasons that are really
/// one. First, this is the true reachable ceiling: the frontend RPC client
/// clamps every per-call timeout to `PER_CALL_TIMEOUT_MAX_MS = 600 s`
/// (`app/src/services/coreRpcClient.ts`), so a server budget above that can never
/// be observed — the client aborts first. Second, that cap also bounds the
/// blocking-pool worker this budget guards: `max_sessions` is untrusted (an
/// advertised programmatic RPC, `platform/about_app/catalog_data.rs`), and
/// without the cap a caller passing 1000 would pin a thread for ~33 h. Capping
/// the resulting `Duration` — not the multiplier — makes both true at once.
///
/// Because a single pass is bounded, large histories drain across repeated passes
/// (client `drainCodingSessions`); the per-pass batch is sized so `BASE + N*PER`
/// stays under the cap for the UI's `CODING_SESSION_BATCH_MAX`, keeping the
/// server budget the *tighter* of the two so it returns a clean structured
/// timeout before the client's fetch aborts. This is a *ceiling to catch a wedged
/// run*, not a latency target.
///
/// `pub(crate)` so the module driver can size its own bus deadline from the
/// same formula (`modules::memory`). That call sits *inside* this one, and
/// tinybus gives every call a 30 s default deadline if nobody sets one — 19×
/// tighter than the smallest budget computed here, which is how a completed
/// import came to be reported as a failure (#5802). One formula, two layers.
pub(crate) fn ingest_budget(max_sessions: usize) -> std::time::Duration {
    /// Fixed overhead allowance (config load, discovery, process warm-up) added
    /// on top of the per-session budget.
    const BASE_SECS: u64 = 120;
    /// Per-session allowance, sized for ~3 sequential per-window LLM calls at the
    /// 20–45 s/window span observed in #5509 rather than the single call the old
    /// formula assumed.
    const PER_SESSION_SECS: u64 = 90;
    /// Hard ceiling on the whole budget. Mirrors the frontend's
    /// `PER_CALL_TIMEOUT_MAX_MS` (600 s) — a larger budget is unreachable because
    /// the client aborts first — and bounds the blocking worker against an
    /// untrusted `max_sessions`.
    const HARD_CAP_SECS: u64 = 600;

    let scaled = BASE_SECS.saturating_add((max_sessions as u64).saturating_mul(PER_SESSION_SECS));
    std::time::Duration::from_secs(scaled.min(HARD_CAP_SECS))
}

/// Distil local coding-agent transcripts into observations.
///
/// The pipeline runs driver-side now, which removes the blocking-worker hop
/// this handler used to need: the engine's persona pass carried borrowed path
/// state and was not `Send`, so it had to be driven from `spawn_blocking` with
/// a `block_on` inside. The contract member is an ordinary `Send` future, so
/// the RPC simply awaits it.
///
/// **The deadline stays here on purpose.** `MemoryCodingSessions` documents
/// that it cannot bound the wall-clock cost — each session is one or more
/// sequential model calls — and that a caller needing a deadline enforces it on
/// its own side. [`ingest_budget`] is that deadline, unchanged; a timeout is
/// still reported as a structured error rather than as a short report, because
/// a report the run never finished writing is not progress the caller can keep.
pub async fn ingest_coding_sessions_rpc(
    req: CodingSessionIngestRequest,
) -> Result<Outcome<CodingSessionIngestReport>, String> {
    tracing::info!(
        backfill = req.backfill,
        max_sessions = req.max_sessions,
        "[memory_sources] ingest_coding_sessions_rpc: entry"
    );
    let config = Config::load_or_init()
        .await
        .map_err(|error| format!("load config for coding-session ingestion: {error}"))?;
    let binding = crate::memory::binding::for_config(&config)?;
    let Some(sessions) = binding.provider().as_coding_sessions() else {
        return Err(unserved(
            &binding,
            "coding sessions",
            "ingest_coding_sessions",
        ));
    };

    // Check the existing consent/provider ladder before any transcript can be
    // sent to inference. This reports a missing step without changing settings.
    validate_ingestion_provider(&config)?;

    // Wall-clock ceiling so a stalled provider call or a wedged session step
    // can't keep the RPC waiting indefinitely (#4863 review), sized to the
    // requested backfill so a legitimate large run isn't killed mid-flight
    // while a genuine infinite hang still terminates. Read before `req` moves.
    let ingest_timeout = ingest_budget(req.max_sessions);
    let report = tokio::time::timeout(ingest_timeout, sessions.ingest_coding_sessions(req))
        .await
        .map_err(|_elapsed| {
            tracing::error!(
                driver = %binding.driver_id(),
                timeout_secs = ingest_timeout.as_secs(),
                "[memory_sources] ingest_coding_sessions_rpc: timed out"
            );
            format!(
                "ingest coding sessions: timed out after {}s",
                ingest_timeout.as_secs()
            )
        })?
        .map_err(|error| format!("ingest coding sessions: {error}"))?;

    tracing::info!(
        driver = %binding.driver_id(),
        mode = %report.mode,
        processed = report.sessions_processed,
        failed = report.sessions_failed,
        budget_hit = report.budget_hit,
        "[memory_sources] ingest_coding_sessions_rpc: exit"
    );
    Ok(Outcome::new(report, vec![]))
}

/// Refuse missing processing prerequisites without changing consent or routing.
fn validate_ingestion_provider(config: &Config) -> Result<(), String> {
    if !config.local_ai.runtime_enabled && !config.memory_tree.cloud_summarization_opt_in {
        return Err("cloud_processing_disabled: enable local AI, or explicitly opt in to cloud memory processing before importing sessions".into());
    }
    let (available, _) = crate::memory::tree::tree_runtime::ops::summarizer_available(config);
    if !available {
        return Err("summarization_unavailable: configure a summarisation provider in Connections, or check that your local AI model is available".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "coding_sessions_tests.rs"]
mod tests;
