//! The hosted engine's "background work is free" predicate.
//!
//! Background memory jobs that write a user's whole memory again start on
//! their own only when that costs the user nothing. For the hosted
//! `tinyhumans` engine that is true only while the backend's memory free
//! period is on (`GET /memory/free-period`). The answer is cached for
//! [`CACHE_TTL`] per backend and account.
//!
//! Anything unknown (signed out, an error, a backend without the route) is
//! not free, so nothing starts on its own.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::memory_engines::BoundEngine;
use openhuman_embed::__host::security::credentials::session_support::{
    resolve_backend_credential, BackendCredential,
};
use sha2::Digest;

use crate::client::{flatten_authed_error, BackendClient};

/// How long an answer is reused. A job polls between batches; the period
/// changes on the order of days, so a minute keeps it to one request a minute.
pub(crate) const CACHE_TTL: Duration = Duration::from_secs(60);

/// The last answer for each backend. One async lock covers the lookup and the
/// fetch on a miss, so concurrent callers (two jobs polling at once) wait for
/// one request rather than each sending their own.
pub(crate) struct Answer {
    inner: tokio::sync::Mutex<Option<HashMap<String, (Instant, bool)>>>,
}

impl Answer {
    pub(crate) const fn new() -> Self {
        Self {
            inner: tokio::sync::Mutex::const_new(None),
        }
    }
}

static ANSWER: Answer = Answer::new();

/// Whether memory work is free for the user right now on the hosted engine.
pub(crate) async fn free_period_active(config: Config, bound: BoundEngine) -> bool {
    let Ok(credential) = resolve_backend_credential(&config) else {
        tracing::debug!("[tinyhumans][memory] free period: no credential; not free");
        return false;
    };
    // The answer is the account's on the backend memory is bound to, so the
    // cache is keyed by both: one account's answer never serves another.
    let key = format!("{}|{}", bound.endpoint, digest(credential.secret()));
    active_with_cache(&ANSWER, &key, CACHE_TTL, Instant::now, || {
        fetch(&credential, &bound.endpoint)
    })
    .await
}

/// A one-way digest of a credential for a cache key: the full SHA-256, so
/// two accounts never share a key.
pub(crate) fn digest(secret: &str) -> String {
    sha2::Sha256::digest(secret.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Asks `backend` whether its memory free period is on for `credential`.
pub(crate) async fn fetch(credential: &BackendCredential, backend: &str) -> Result<bool, String> {
    let client = BackendClient::new(backend).map_err(|e| format!("{e:#}"))?;
    let data = client
        .authed_json(
            credential.clone(),
            reqwest::Method::GET,
            "/memory/free-period",
            None,
        )
        .await
        .map_err(flatten_authed_error)?;
    parse_active(&data)
}

/// `active` from the route's answer; anything else is an error. The
/// transport unwraps the `{success, data}` envelope, but an answer that
/// still carries it is read the same way.
pub(crate) fn parse_active(data: &serde_json::Value) -> Result<bool, String> {
    data.get("data")
        .unwrap_or(data)
        .get("active")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| "free-period answer has no boolean `active`".to_string())
}

/// The cached answer for `key`, else `fetch`'s, with a failure read as not
/// free. Failures are cached too, so a backend without the route is asked
/// once a minute, not once a batch.
pub(crate) async fn active_with_cache<F, Fut>(
    cache: &Answer,
    key: &str,
    ttl: Duration,
    clock: impl Fn() -> Instant,
    fetch: F,
) -> bool
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<bool, String>>,
{
    let mut answers = cache.inner.lock().await;
    let answers = answers.get_or_insert_with(HashMap::new);
    // Read after the lock: a wait behind another backend's fetch must not
    // stretch a cached answer past its TTL.
    let now = clock();
    if let Some((_, active)) = answers
        .get(key)
        .filter(|(at, _)| now.duration_since(*at) < ttl)
    {
        return *active;
    }
    let active = match fetch().await {
        Ok(active) => active,
        Err(_) => {
            // The error text can carry the backend's response; it is not
            // logged.
            tracing::debug!("[tinyhumans][memory] free period unknown; treating as not free");
            false
        }
    };
    // The answer's age starts when it arrives, not when it was asked for.
    answers.insert(key.to_string(), (clock(), active));
    active
}
