//! The sandbox scopes the host has registered with the module.
//!
//! The module holds a bounded table of scopes and never evicts one, so the host
//! keeps its own bounded, least-recently-used view and retires what it drops. A
//! retirement invalidates every plan the scope issued, which can only refuse a
//! spawn, never widen one. Registering is idempotent in the module, so a stale entry
//! (the module restarted) costs one refused plan and one re-registration.

use std::collections::VecDeque;
use std::sync::OnceLock;

use tinysecurity_bus::{RegisterSandboxScopeRequest2, SandboxScope2};

use super::{unavailable, PlanError};
use crate::modules::security::sandbox as client;

/// Scopes the host keeps registered at once. Each distinct set of facts (another
/// action dir, another mount set) is one scope, so a long-running core would otherwise
/// accumulate them without bound.
const LIVE_LIMIT: usize = 256;

/// Registered scope identities, least recently used first.
fn live() -> &'static tokio::sync::Mutex<VecDeque<String>> {
    static LIVE: OnceLock<tokio::sync::Mutex<VecDeque<String>>> = OnceLock::new();
    LIVE.get_or_init(|| tokio::sync::Mutex::new(VecDeque::new()))
}

/// Make the module hold `scope` on `audit_stream_id`, registering it on first use.
pub(super) async fn ensure(
    proxy: &tinybus::Proxy,
    scope: &SandboxScope2,
    audit_stream_id: &str,
) -> Result<(), PlanError> {
    let mut live = live().lock().await;
    if let Some(at) = live.iter().position(|id| id == &scope.scope_id) {
        let id = live.remove(at).unwrap_or_default();
        live.push_back(id);
        return Ok(());
    }
    while live.len() >= LIVE_LIMIT {
        let Some(oldest) = live.pop_front() else {
            break;
        };
        tracing::debug!("[sandbox:plan] retiring the least recently used sandbox scope");
        if let Err(error) = client::retire_scope(proxy, &oldest).await {
            // The module does not know it (it restarted) or cannot retire it. Either way
            // the host stops relying on it: a plan under it is refused by the module, and
            // registering it again is idempotent. The module's own bound still holds, so
            // a module that keeps failing runs out of room and refuses new scopes.
            tracing::warn!(%error, "[sandbox:plan] could not retire a sandbox scope; dropping it");
        }
    }
    let receipt = client::register_scope(
        proxy,
        RegisterSandboxScopeRequest2 {
            scope: scope.clone(),
            audit_stream_id: audit_stream_id.to_owned(),
            report_handle: false,
        },
    )
    .await
    .map_err(unavailable)?;
    tracing::debug!(
        outcome = ?receipt.outcome,
        live_scopes = receipt.live_scopes,
        "[sandbox:plan] sandbox scope current"
    );
    live.push_back(scope.scope_id.clone());
    Ok(())
}

/// Drop a scope the module no longer answers for, so the next spawn registers it again.
pub(super) async fn forget(scope_id: &str) {
    live().lock().await.retain(|id| id != scope_id);
}
