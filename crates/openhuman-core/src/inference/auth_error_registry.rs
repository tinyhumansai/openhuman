//! Process-lived registry of BYO provider auth failures (invalid / revoked
//! API key, HTTP 401 / 403).
//!
//! Single source of truth feeding both user-facing surfaces for a rejected
//! BYO key, so the two can never drift:
//!   - the **notification center** — a one-shot [`crate::core::bus::
//!     DomainEvent::ProviderApiKeyRejected`] published the first time a
//!     provider starts failing, and
//!   - the **AI-settings provider-error notice** — read live via the
//!     `openhuman.inference_provider_auth_errors` RPC ([`snapshot`]).
//!
//! Entries are recorded at the demote site
//! ([`provider::http_error::log_byo_provider_auth_failure`](super::provider::http_error::log_byo_provider_auth_failure)) and cleared
//! when the user updates or removes that provider's key
//! (`credentials::ops`). The [`record`] latch is what makes the notification
//! fire **once per failure episode** rather than once per retry: the
//! triggering 401 repeats thousands of times (the memory-summarization loop
//! re-attempts on every scoring pass — TAURI-RUST-4RC: ~9k events / 6 users),
//! and an unguarded publish would re-flood the notification center the same
//! way the raw error flooded Sentry.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tinyinference_providers::auth_registry::ProviderAuthError;
use tinyinference_providers::auth_registry::ProviderAuthErrorRegistry;

/// The process-lived registry. Episode tracking lives in
/// `tinyinference-providers`; this module supplies the process singleton, the
/// user-facing copy and the clock.
fn registry() -> &'static ProviderAuthErrorRegistry {
    static REG: OnceLock<ProviderAuthErrorRegistry> = OnceLock::new();
    REG.get_or_init(ProviderAuthErrorRegistry::new)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Build the actionable, user-facing message for a BYO key the provider
/// rejected. Phrased so it reads the same in the notification center and the
/// AI-settings notice, and points the user at the one fix that resolves it
/// (the third-party key is invalid / revoked — OpenHuman has no other lever).
pub fn auth_error_message(provider: &str, status: u16) -> String {
    format!(
        "{provider} rejected the API key (HTTP {status}). Update your {provider} \
         API key in Connections → API keys → LLM to restore it."
    )
}

/// Record a BYO provider auth failure.
///
/// Returns `true` when this is a **new** failure episode for `provider` (no
/// prior entry) — callers publish the one-shot notification only on `true` so
/// a 401 that repeats per-retry doesn't re-notify. An existing entry is
/// refreshed (status / timestamp) and returns `false`.
pub fn record(provider: &str, status: u16) -> bool {
    record_in_scope(provider, status, current_scope())
}

fn record_in_scope(provider: &str, status: u16, scope: CurrentScope) -> bool {
    if let CurrentScope::Unresolved(error) = scope {
        log::warn!("[auth-errors] record provider={provider} skipped: scope unresolved ({error})");
        return false;
    }

    let timestamp_ms = now_ms();
    let changed = registry().record(ProviderAuthError {
        provider: provider.to_string(),
        status,
        message: auth_error_message(provider, status),
        timestamp_ms,
    });
    let mut origins = origins().lock().unwrap_or_else(|error| error.into_inner());
    match scope {
        CurrentScope::Bound(scope) => {
            origins
                .entry(provider.to_owned())
                .or_default()
                .scopes
                .insert(
                    scope,
                    ScopedFailure {
                        status,
                        timestamp_ms,
                    },
                );
        }
        CurrentScope::Unbound => {
            origins.entry(provider.to_owned()).or_default().unscoped = true;
        }
        CurrentScope::Unresolved(_) => unreachable!("unresolved scope returned before recording"),
    }
    changed
}

/// Clear the recorded auth error for `provider` in the current scope (the
/// user updated / removed the key, or a call newly succeeded). Another
/// scope's failure for the same provider is its own key and stays. Returns
/// `true` if the entry was removed, which re-arms the [`record`] latch so a
/// fresh failure re-notifies.
pub fn clear(provider: &str) -> bool {
    let mut origins = origins().lock().unwrap_or_else(|error| error.into_inner());
    match current_scope() {
        CurrentScope::Unbound => {}
        CurrentScope::Unresolved(error) => {
            log::warn!(
                "[auth-errors] clear provider={provider} skipped: scope unresolved ({error})"
            );
            return false;
        }
        CurrentScope::Bound(scope) => {
            if let Some(origin) = origins.get_mut(provider) {
                origin.scopes.remove(&scope);
                if !origin.scopes.is_empty() || origin.unscoped {
                    return false;
                }
            }
        }
    }
    origins.remove(provider);
    registry().clear(provider)
}

/// Snapshot all currently-recorded provider auth errors, sorted by provider
/// slug for a stable RPC payload.
pub fn snapshot() -> Vec<ProviderAuthError> {
    registry().snapshot()
}

/// The auth errors the current workspace/tenant may see: those it recorded,
/// with its own status and time. Without a bound context (a single-user
/// process) every error is its own; a bound context whose workspace can't be
/// resolved gets an error rather than every scope's failures.
pub fn snapshot_current_scope() -> Result<Vec<ProviderAuthError>, String> {
    let scope = match current_scope() {
        CurrentScope::Unbound => return Ok(snapshot()),
        CurrentScope::Unresolved(error) => return Err(error),
        CurrentScope::Bound(scope) => scope,
    };
    let origins = origins().lock().unwrap_or_else(|error| error.into_inner());
    Ok(registry()
        .snapshot()
        .into_iter()
        .filter_map(|error| {
            let own = origins.get(&error.provider)?.scopes.get(&scope)?;
            Some(own.view(error))
        })
        .collect())
}

#[derive(Default)]
struct ErrorOrigins {
    /// Each scope's own latest failure for the provider.
    scopes: HashMap<ScopeIdentity, ScopedFailure>,
    unscoped: bool,
}

#[derive(Clone, Copy)]
struct ScopedFailure {
    status: u16,
    timestamp_ms: u64,
}

impl ScopedFailure {
    /// `error` as this scope saw it.
    fn view(self, error: ProviderAuthError) -> ProviderAuthError {
        ProviderAuthError {
            message: auth_error_message(&error.provider, self.status),
            status: self.status,
            timestamp_ms: self.timestamp_ms,
            provider: error.provider,
        }
    }
}

enum CurrentScope {
    /// No core context is bound (a single-user process).
    Unbound,
    /// A bound context whose workspace binding could not be read.
    Unresolved(String),
    Bound(ScopeIdentity),
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ScopeIdentity {
    workspace: std::path::PathBuf,
    config_path: Option<std::path::PathBuf>,
    tenant: String,
}

fn origins() -> &'static Mutex<HashMap<String, ErrorOrigins>> {
    static ORIGINS: OnceLock<Mutex<HashMap<String, ErrorOrigins>>> = OnceLock::new();
    ORIGINS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn current_scope() -> CurrentScope {
    let saas = crate::core::runtime::is_saas();
    current_scope_in(saas, crate::core::runtime::tenant::context_in(saas))
}

fn current_scope_in(
    saas: bool,
    context: Option<std::sync::Arc<crate::core::runtime::CoreContext>>,
) -> CurrentScope {
    let Some(context) = context else {
        return if saas {
            CurrentScope::Unresolved("no profile context in SaaS mode".to_owned())
        } else {
            CurrentScope::Unbound
        };
    };
    match context.workspace_config_binding() {
        Ok((workspace, config_path)) => CurrentScope::Bound(ScopeIdentity {
            workspace,
            config_path,
            tenant: context.profile().unwrap_or_default().to_owned(),
        }),
        Err(error) => CurrentScope::Unresolved(error),
    }
}

/// Auth failures scoped to the current workspace/tenant. Known foreign
/// failures are omitted; unscoped legacy failures are reported separately so
/// callers can avoid attributing them to the selected profile.
pub fn snapshot_for_scope(
    workspace: &std::path::Path,
    config_path: &std::path::Path,
    tenant: Option<&str>,
) -> (Vec<ProviderAuthError>, bool) {
    let scope = ScopeIdentity {
        workspace: workspace.to_path_buf(),
        config_path: Some(config_path.to_path_buf()),
        tenant: tenant.unwrap_or_default().to_owned(),
    };
    let origins = origins().lock().unwrap_or_else(|error| error.into_inner());
    let mut has_unscoped = origins.values().any(|origin| origin.unscoped);
    let errors = registry()
        .snapshot()
        .into_iter()
        .filter_map(|error| {
            let origin = origins.get(&error.provider)?;
            let partial_match = origin.scopes.keys().any(|recorded| {
                recorded.config_path.is_none()
                    && recorded.workspace == scope.workspace
                    && recorded.tenant == scope.tenant
            });
            if partial_match {
                has_unscoped = true;
            }
            // This scope's own status and time, not another scope's latest.
            Some(origin.scopes.get(&scope)?.view(error))
        })
        .collect();
    (errors, has_unscoped)
}

/// Test-only: wipe the registry so a test can't be flaked by an entry an
/// earlier test left behind (the map is process-global).
#[cfg(any(test, debug_assertions))]
pub fn reset_for_tests() {
    registry().clear_all();
    origins()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
}

#[cfg(test)]
#[path = "auth_error_registry_tests.rs"]
mod tests;
