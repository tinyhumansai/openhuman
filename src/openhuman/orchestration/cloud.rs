//! Hosted-brain uplink + read surface.
//!
//! Forwards sanitized orchestration events (`POST /orchestration/v1/events`) and
//! world-diff batches (`POST /orchestration/v1/world-diff`) to the hosted brain,
//! which runs the wake/reasoning graph server-side, and reads back
//! sessions / messages / steering (`GET /orchestration/v1/*`). Forwarding is
//! best-effort and fire-and-forget, so it never blocks or fails ingest.
//!
//! Auth + base-URL plumbing mirrors the other hosted-API adapters
//! (`announcements/ops.rs`, `billing/ops.rs`): an app-session JWT via
//! `require_live_session_token`, the backend base via `effective_backend_api_url`,
//! and one shared `BackendOAuthClient`.

use std::time::Duration;

use reqwest::Method;
use serde_json::Value;

use crate::api::config::effective_backend_api_url;
use crate::api::BackendOAuthClient;
use crate::openhuman::config::Config;

use super::wire::{OrchestrationEventEnvelopeWire, WorldDiffBatchWire};

const LOG: &str = "orchestration";
const EVENTS_PATH: &str = "/orchestration/v1/events";
const WORLD_DIFF_PATH: &str = "/orchestration/v1/world-diff";

/// Jittered retry schedule for a transient push failure (3 retries after the
/// first attempt). Matches the plan's 1s/4s/10s cadence.
const DEFAULT_BACKOFFS: [Duration; 3] = [
    Duration::from_secs(1),
    Duration::from_secs(4),
    Duration::from_secs(10),
];

/// Push one sanitized event to the hosted brain. Resolves the app-session JWT
/// and backend base, then POSTs with bounded retry. Returns `Err` only after
/// the retry budget is exhausted (or the session is signed out).
pub async fn push_event(
    config: &Config,
    envelope: &OrchestrationEventEnvelopeWire,
) -> Result<Option<String>, String> {
    let token = crate::openhuman::credentials::session_support::require_live_session_token(config)?;
    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    push_event_with(&client, &token, envelope, &DEFAULT_BACKOFFS).await
}

/// Upload a batch of world-diff entries — the subconscious tier's primary
/// trigger. Same auth/base/retry plumbing as [`push_event`]. Returns `Err` only
/// after the retry budget is exhausted (or the session is signed out).
pub async fn push_world_diff(config: &Config, batch: &WorldDiffBatchWire) -> Result<(), String> {
    if batch.entries.is_empty() {
        return Ok(());
    }
    let token = crate::openhuman::credentials::session_support::require_live_session_token(config)?;
    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    post_with_retry(
        &client,
        &token,
        WORLD_DIFF_PATH,
        batch.to_value(),
        &DEFAULT_BACKOFFS,
        &format!(
            "world-diff session={} entries={}",
            batch.session_id,
            batch.entries.len()
        ),
    )
    .await
    .map(|_| ())
}

/// Inner push with an injectable client, token, and backoff schedule so the
/// transport can be exercised against a mock server without real credentials or
/// real sleeps (`backoffs = &[]` → single attempt). Public for integration
/// tests (`tests/orchestration_shadow_push_e2e.rs`).
pub async fn push_event_with(
    client: &BackendOAuthClient,
    token: &str,
    envelope: &OrchestrationEventEnvelopeWire,
    backoffs: &[Duration],
) -> Result<Option<String>, String> {
    let label = format!(
        "event session={} seq={}",
        envelope.session_id, envelope.event.seq
    );
    let resp = post_with_retry(
        client,
        token,
        EVENTS_PATH,
        envelope.to_value(),
        backoffs,
        &label,
    )
    .await?;
    // `202 { accepted, cycleId }` — return the cycleId so the caller can record
    // the cycle's device-authoritative origin (see `exec_gate`).
    Ok(resp
        .get("cycleId")
        .and_then(|v| v.as_str())
        .map(str::to_string))
}

/// Generic authed POST with bounded jittered-backoff retry. Shared by every
/// orchestration uplink (`events`, `world-diff`). `backoffs = &[]` → one attempt.
async fn post_with_retry(
    client: &BackendOAuthClient,
    token: &str,
    path: &str,
    body: serde_json::Value,
    backoffs: &[Duration],
    label: &str,
) -> Result<Value, String> {
    let mut attempt: usize = 0;
    loop {
        match client
            .authed_json(token, Method::POST, path, Some(body.clone()))
            .await
        {
            Ok(data) => {
                log::debug!(target: LOG, "[orchestration] cloud.push.ok {label} attempt={}", attempt + 1);
                return Ok(data);
            }
            Err(err) => {
                let msg = crate::api::flatten_authed_error(err);
                if attempt >= backoffs.len() {
                    log::warn!(target: LOG, "[orchestration] cloud.push.give_up {label} attempts={} err={msg}", attempt + 1);
                    return Err(msg);
                }
                log::warn!(target: LOG, "[orchestration] cloud.push.retry {label} attempt={} err={msg}", attempt + 1);
                tokio::time::sleep(backoffs[attempt]).await;
                attempt += 1;
            }
        }
    }
}

/// World-diff uploader with injectable client/token/backoffs for tests.
pub async fn push_world_diff_with(
    client: &BackendOAuthClient,
    token: &str,
    batch: &WorldDiffBatchWire,
    backoffs: &[Duration],
) -> Result<(), String> {
    let label = format!(
        "world-diff session={} entries={}",
        batch.session_id,
        batch.entries.len()
    );
    post_with_retry(
        client,
        token,
        WORLD_DIFF_PATH,
        batch.to_value(),
        backoffs,
        &label,
    )
    .await
    .map(|_| ())
}

// ── Hosted read surface (GET) ─────────────────────────────────────────────────
// The renderer reads session / message / state / steering / world-diff state
// from the hosted brain over these routes. Each returns the unwrapped `data`
// payload (`BackendOAuthClient` strips the `{success,data}` envelope). Callers
// fall back to the local render cache on `Err` and surface an offline notice.
// A replayed/duplicate write is a 202 (not a 409) server-side, so a read never
// needs to special-case dedupe.

const SESSIONS_PATH: &str = "/orchestration/v1/sessions";
const STEERING_PATH: &str = "/orchestration/v1/steering";

/// A session token + backend client resolved once and reused across every GET in a
/// single sync read pass. `sync_reads` issues `fetch_sessions` + up to
/// `MAX_SESSIONS_PER_SYNC` `fetch_messages` + `fetch_steering` per 20s tick, so
/// rebuilding the client (and re-loading the session profile) per GET would repeat
/// the profile lookup and discard the reqwest connection pool on every request.
/// Resolve it once with [`read_pass`] and thread it through the pass.
pub struct ReadPass {
    client: BackendOAuthClient,
    token: String,
}

/// Resolve the token + client for one sync read pass. `Err` (no live session) → the
/// caller degrades the whole pass to the local render cache.
pub fn read_pass(config: &Config) -> Result<ReadPass, String> {
    let token = crate::openhuman::credentials::session_support::require_live_session_token(config)?;
    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    Ok(ReadPass { client, token })
}

impl ReadPass {
    /// GET the hosted session list →
    /// `[{ sessionId, counterpartAgentId, lastSeq, status, lastCycleId?, lastEventTs?, updatedAt? }]`.
    pub async fn fetch_sessions(&self) -> Result<Value, String> {
        self.authed_get(SESSIONS_PATH.to_string(), "sessions").await
    }

    /// GET messages for a session, optionally after a `seq` cursor →
    /// `[{ seq, role, sender, body, kind, ts, cycleId }]`. Cursor param is `?after=`.
    pub async fn fetch_messages(
        &self,
        session_id: &str,
        after_seq: Option<i64>,
    ) -> Result<Value, String> {
        let mut path = format!(
            "/orchestration/v1/sessions/{}/messages",
            urlencoding::encode(session_id)
        );
        if let Some(after) = after_seq {
            path.push_str(&format!("?after={after}"));
        }
        self.authed_get(path, "messages").await
    }

    /// GET the current steering directive + recent history →
    /// `{ active: { directive, consumedCycles, maxCycles } | null, history: [{ directive, createdAt? }] }`.
    pub async fn fetch_steering(&self) -> Result<Value, String> {
        self.authed_get(STEERING_PATH.to_string(), "steering").await
    }

    /// Shared authed GET → unwrapped `data`, reusing this pass's token + client.
    /// Returns a flattened error string on transport / non-2xx so the caller can
    /// degrade to the local render cache.
    async fn authed_get(&self, path: String, label: &str) -> Result<Value, String> {
        match self
            .client
            .authed_json(&self.token, Method::GET, &path, None)
            .await
        {
            Ok(data) => {
                log::debug!(target: LOG, "[orchestration] cloud.read.ok {label}");
                Ok(data)
            }
            Err(err) => {
                let msg = crate::api::flatten_authed_error(err);
                log::warn!(target: LOG, "[orchestration] cloud.read.fail {label} err={msg}");
                Err(msg)
            }
        }
    }
}

// Transport tests live in `tests/orchestration_shadow_push_e2e.rs` (integration
// crate): the root crate's `cfg(test)` build is currently blocked by unrelated
// stale test modules at this checkout, so the pusher is exercised over wiremock
// from an integration test that links the compiled lib instead.
