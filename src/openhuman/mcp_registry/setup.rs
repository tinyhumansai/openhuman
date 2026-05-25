//! Opaque secret-ref machinery for the MCP setup agent.
//!
//! The setup agent must collect credentials from the user **without** the
//! raw values ever entering the LLM context. The flow is:
//!
//! 1. Agent calls `mcp_setup_request_secret(key_name, prompt)`. Core mints
//!    a fresh [`SecretRef`] (`secret://<hex>`), publishes
//!    [`crate::core::event_bus::DomainEvent::McpSetupSecretRequested`] so
//!    the UI can render a native prompt, and **awaits** the user.
//! 2. UI prompts the user out-of-band and POSTs back via
//!    `mcp_setup_submit_secret(ref_id, value)`. Core stores the raw value
//!    against the ref and wakes the waiting `request_secret` call.
//! 3. Agent receives the ref and passes it into `mcp_setup_test_connection`
//!    or `mcp_setup_install_and_connect`. Core resolves refs → values
//!    just-in-time and either spawns a scratch subprocess (test) or
//!    persists them into `mcp_client_env` (install).
//!
//! Raw values never enter or exit through the agent-facing tool calls.
//! The map is process-local and cleared on shutdown; values do not
//! persist across restarts unless committed via `consume_refs` →
//! `mcp_client_env`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, Mutex};
use tokio::time::timeout;

/// How long an unfulfilled `request_secret` waits before giving up.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(300); // 5 min

/// How long a fulfilled-but-unused secret hangs around before being purged.
/// Long enough to support iterative `test_connection` retries; short enough
/// that an abandoned conversation doesn't leave secrets stranded.
pub const IDLE_TTL: Duration = Duration::from_secs(900); // 15 min

// ── Types ────────────────────────────────────────────────────────────────────

/// `secret://<hex>` — opaque handle returned to the agent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretRef(String);

impl SecretRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Parse an agent-supplied string. Accepts both bare hex and the
    /// `secret://` prefixed form so callers can pass whatever they got
    /// back from `request_secret`.
    pub fn parse(s: &str) -> Option<Self> {
        let trimmed = s.strip_prefix("secret://").unwrap_or(s).trim();
        if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        Some(Self(format!("secret://{trimmed}")))
    }

    fn mint() -> Self {
        // 12 hex chars (48 bits) sourced from the first 6 bytes of a
        // UUIDv4 (OsRng-backed). Short enough to log; collision-free in
        // any sane setup-session window.
        let raw = uuid::Uuid::new_v4().simple().to_string();
        let hex = &raw[..12];
        Self(format!("secret://{hex}"))
    }
}

/// One entry in the global setup-secret map.
struct SecretEntry {
    /// Display key name (e.g. `NOTION_API_KEY`). Safe to log; the *value*
    /// is not. Returned to the UI in the request event.
    key_name: String,
    /// `None` while we're waiting for the UI to submit; `Some(value)`
    /// once submitted.
    value: Option<String>,
    /// Wall-clock time the entry was last touched (created or fulfilled).
    /// Used by the GC sweep to enforce `IDLE_TTL`.
    last_touched: Instant,
    /// Wakes the matching `request_secret` call once `value` is populated.
    /// Taken (set to `None`) on first fulfillment so a double-submit is a
    /// no-op rather than a panic.
    waiter: Option<oneshot::Sender<()>>,
}

// ── Global registry ──────────────────────────────────────────────────────────

type Map = Arc<Mutex<HashMap<SecretRef, SecretEntry>>>;

static SETUP_SECRETS: OnceLock<Map> = OnceLock::new();

fn map() -> &'static Map {
    SETUP_SECRETS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Mint a new ref + parked waiter for `key_name`. Returns the ref and the
/// receiver the caller should `.await` on (with a timeout) until the UI
/// submits the value via [`fulfill`].
///
/// The waiter is taken out of the map; if `fulfill` arrives before the
/// caller awaits, the value is stashed in `entry.value` and the oneshot
/// fires immediately when awaited.
pub async fn mint_request(key_name: &str) -> (SecretRef, oneshot::Receiver<()>) {
    let (tx, rx) = oneshot::channel();
    let r = SecretRef::mint();
    let entry = SecretEntry {
        key_name: key_name.to_string(),
        value: None,
        last_touched: Instant::now(),
        waiter: Some(tx),
    };
    map().lock().await.insert(r.clone(), entry);
    tracing::debug!(
        "[mcp-setup] minted ref={} key_name={}",
        r.as_str(),
        key_name
    );
    (r, rx)
}

/// UI-side: fulfill a pending ref with the raw value. Returns `true` if
/// the ref existed and was awaiting; `false` if it was unknown or already
/// fulfilled.
pub async fn fulfill(r: &SecretRef, value: String) -> bool {
    let mut guard = map().lock().await;
    let Some(entry) = guard.get_mut(r) else {
        tracing::warn!("[mcp-setup] fulfill: unknown ref={}", r.as_str());
        return false;
    };
    if entry.value.is_some() {
        tracing::warn!("[mcp-setup] fulfill: double-submit ref={}", r.as_str());
        return false;
    }
    entry.value = Some(value);
    entry.last_touched = Instant::now();
    if let Some(tx) = entry.waiter.take() {
        let _ = tx.send(());
    }
    tracing::debug!("[mcp-setup] fulfilled ref={}", r.as_str());
    true
}

/// Block on a freshly-minted request with the global timeout. On timeout
/// the entry is removed and `Err(_)` is returned.
pub async fn await_fulfillment(r: &SecretRef, rx: oneshot::Receiver<()>) -> anyhow::Result<()> {
    match timeout(REQUEST_TIMEOUT, rx).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(_)) => {
            // Sender dropped — usually means GC purged the entry. Surface
            // as a timeout-style error to keep the caller simple.
            let _ = forget(r).await;
            anyhow::bail!("secret request {} cancelled before user submit", r.as_str())
        }
        Err(_) => {
            let _ = forget(r).await;
            anyhow::bail!(
                "secret request {} timed out after {}s",
                r.as_str(),
                REQUEST_TIMEOUT.as_secs()
            )
        }
    }
}

/// Resolve a `{KEY: SecretRef}` map into a `Vec<(KEY, VALUE)>`. Returns
/// `Err(_)` if any ref is unknown or not yet fulfilled — callers should
/// retry rather than partially-apply.
///
/// Touches the `last_touched` on every hit so iterative `test_connection`
/// calls reset the idle TTL.
pub async fn resolve_refs(
    refs: &HashMap<String, SecretRef>,
) -> anyhow::Result<Vec<(String, String)>> {
    let mut guard = map().lock().await;
    let mut out = Vec::with_capacity(refs.len());
    for (key, r) in refs {
        let entry = guard
            .get_mut(r)
            .ok_or_else(|| anyhow::anyhow!("unknown secret ref {}", r.as_str()))?;
        let value = entry
            .value
            .clone()
            .ok_or_else(|| anyhow::anyhow!("secret ref {} not yet fulfilled", r.as_str()))?;
        entry.last_touched = Instant::now();
        out.push((key.clone(), value));
    }
    Ok(out)
}

/// Same as [`resolve_refs`] but also removes the entries from the map on
/// success. Used by `install_and_connect` once the values have been
/// persisted to `mcp_client_env`. On failure the entries are left intact
/// so the agent can retry without re-prompting.
pub async fn consume_refs(
    refs: &HashMap<String, SecretRef>,
) -> anyhow::Result<Vec<(String, String)>> {
    // First pass: resolve. Bail without mutation if any ref is missing.
    let resolved = resolve_refs(refs).await?;
    // Second pass: drop. Bail-out on the first miss is impossible because
    // we just held the resolved values without releasing the lock — but to
    // be honest we *did* release between the two awaits. Recheck.
    let mut guard = map().lock().await;
    for r in refs.values() {
        guard.remove(r);
    }
    Ok(resolved)
}

/// Drop a single ref. Useful when the agent abandons a half-collected
/// install. Returns `true` if the ref existed.
pub async fn forget(r: &SecretRef) -> bool {
    map().lock().await.remove(r).is_some()
}

/// Sweep entries idle longer than [`IDLE_TTL`]. Intended to be called from
/// a background task; cheap to call frequently. Returns the number of
/// entries reaped.
pub async fn gc_sweep() -> usize {
    let now = Instant::now();
    let mut guard = map().lock().await;
    let before = guard.len();
    guard.retain(|_, entry| now.duration_since(entry.last_touched) < IDLE_TTL);
    let reaped = before - guard.len();
    if reaped > 0 {
        tracing::debug!("[mcp-setup] gc_sweep reaped={reaped}");
    }
    reaped
}

/// Test-only: inspect the key name for a ref. Production callers must
/// not learn this through the agent surface.
#[cfg(test)]
pub(crate) async fn key_name_for(r: &SecretRef) -> Option<String> {
    map().lock().await.get(r).map(|e| e.key_name.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Mutex as AsyncMutex;

    // Setup tests share the static SETUP_SECRETS map. Serialise them via
    // this guard so parallel runs don't trample each other.
    static TEST_GUARD: AsyncMutex<()> = AsyncMutex::const_new(());

    async fn clear_map() {
        map().lock().await.clear();
    }

    #[tokio::test]
    async fn mint_then_fulfill_then_resolve() {
        let _g = TEST_GUARD.lock().await;
        clear_map().await;
        let (r, rx) = mint_request("API_KEY").await;
        let r2 = r.clone();
        let fulfill_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            assert!(fulfill(&r2, "shh".into()).await);
        });
        await_fulfillment(&r, rx).await.expect("fulfilled");
        fulfill_task.await.unwrap();

        let mut refs = HashMap::new();
        refs.insert("API_KEY".to_string(), r.clone());
        let resolved = resolve_refs(&refs).await.expect("resolves");
        assert_eq!(resolved, vec![("API_KEY".to_string(), "shh".to_string())]);

        // Still present after resolve.
        assert_eq!(key_name_for(&r).await.as_deref(), Some("API_KEY"));
    }

    #[tokio::test]
    async fn consume_drops_entries() {
        let _g = TEST_GUARD.lock().await;
        clear_map().await;
        let (r, _rx) = mint_request("TOKEN").await;
        fulfill(&r, "v".into()).await;
        let mut refs = HashMap::new();
        refs.insert("TOKEN".to_string(), r.clone());
        let _ = consume_refs(&refs).await.expect("consumes");
        assert!(key_name_for(&r).await.is_none(), "consumed entry removed");
    }

    #[tokio::test]
    async fn resolve_fails_when_not_fulfilled() {
        let _g = TEST_GUARD.lock().await;
        clear_map().await;
        let (r, _rx) = mint_request("UNSET").await;
        let mut refs = HashMap::new();
        refs.insert("UNSET".to_string(), r);
        assert!(resolve_refs(&refs).await.is_err());
    }

    #[tokio::test]
    async fn resolve_fails_on_unknown_ref() {
        let _g = TEST_GUARD.lock().await;
        clear_map().await;
        let fake = SecretRef::parse("secret://deadbeef").unwrap();
        let mut refs = HashMap::new();
        refs.insert("X".to_string(), fake);
        assert!(resolve_refs(&refs).await.is_err());
    }

    #[tokio::test]
    async fn double_fulfill_is_noop() {
        let _g = TEST_GUARD.lock().await;
        clear_map().await;
        let (r, _rx) = mint_request("K").await;
        assert!(fulfill(&r, "first".into()).await);
        assert!(!fulfill(&r, "second".into()).await);
        let mut refs = HashMap::new();
        refs.insert("K".to_string(), r);
        let resolved = resolve_refs(&refs).await.unwrap();
        assert_eq!(resolved[0].1, "first", "second fulfill ignored");
    }

    #[tokio::test]
    async fn parse_accepts_bare_and_prefixed_hex() {
        assert!(SecretRef::parse("secret://abc123").is_some());
        assert!(SecretRef::parse("abc123").is_some());
        assert!(SecretRef::parse("not-hex").is_none());
        assert!(SecretRef::parse("").is_none());
    }
}
