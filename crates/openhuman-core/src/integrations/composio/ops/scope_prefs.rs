//! Per-toolkit agent scope preferences, held by the connector module.
//!
//! The module stores the rows (`GetUserScopes` / `SetUserScopes`) and applies
//! them when it lists tools and executes actions; this file is the host
//! forwarder the `composio.{get,set}_user_scopes` handlers, the agent's scope
//! gate and the flows capability gate read through.
//!
//! Before the module owned the rows the host kept them in the
//! `composio_user_scopes.json` state value. [`migrate_legacy`]
//! moves any rows still there into the module (once per workspace, on the
//! first read or write) and then empties that file, so a preference a user
//! chose before the move still decides what the agent may run.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use tinyconnectors_bus::names::methods;

use crate::config::Config;
use crate::integrations::composio::contract::UserScopePref;
use crate::integrations::composio::file_store::{self, USER_SCOPES_FILE};
use crate::integrations::composio::module_client as connectors;
use crate::integrations::composio::types::{
    ComposioGetUserScopesRequest, ComposioSetUserScopesRequest, ComposioUserScopes,
    ComposioUserScopesResponse,
};

const LOG: &str = "[composio][scopes]";

/// The legacy file's shape: normalised toolkit slug -> pref.
type LegacyScopeFile = BTreeMap<String, UserScopePref>;

fn to_wire(pref: UserScopePref) -> ComposioUserScopes {
    ComposioUserScopes {
        read: pref.read,
        write: pref.write,
        admin: pref.admin,
    }
}

fn from_wire(scopes: ComposioUserScopes) -> UserScopePref {
    UserScopePref {
        read: scopes.read,
        write: scopes.write,
        admin: scopes.admin,
    }
}

/// Workspaces whose legacy rows have already been moved (or were never there).
fn migrated() -> &'static Mutex<HashSet<PathBuf>> {
    static SET: std::sync::OnceLock<Mutex<HashSet<PathBuf>>> = std::sync::OnceLock::new();
    SET.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Held across a migration pass and across `save`'s write plus its legacy-row
/// drop, so a migration that read the file before a save cannot push its stale
/// row over the value the save just stored.
// ponytail: one process-wide lock; per-workspace if contention ever shows.
static LEGACY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

// Test seam: make the migration of one toolkit's row fail, as a module error
// would, so a deferred (partial) migration can be driven deterministically.
#[cfg(test)]
thread_local! {
    static FAIL_MIGRATING: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Move rows still in the legacy workspace file into the module, then empty
/// the file. A failure leaves the file untouched and is retried on the next
/// call, so no stored preference is lost to a transient module error.
async fn migrate_legacy(ctx: &crate::core::runtime::DispatchCtx, config: &Config) {
    let workspace = config.workspace_dir.clone();
    let done = || {
        migrated()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&workspace)
    };
    // Fast path: a migrated workspace never waits on the lock. Re-checked
    // under it, since another pass may have finished while this one waited.
    if done() {
        return;
    }
    let _legacy = LEGACY.lock().await;
    if done() {
        return;
    }
    let rows = match file_store::load::<LegacyScopeFile>(ctx, config, USER_SCOPES_FILE).await {
        Ok(rows) => rows,
        Err(error) => {
            // Unreadable is not "no rows": keep the file and try again later.
            tracing::warn!(%error, "{LOG} legacy pref file unreadable, migration deferred");
            return;
        }
    };
    for (toolkit, pref) in &rows {
        #[cfg(test)]
        if FAIL_MIGRATING.with(|fail| fail.borrow().as_deref() == Some(toolkit.as_str())) {
            tracing::warn!(%toolkit, "{LOG} legacy pref migration deferred (test seam)");
            return;
        }
        let request = ComposioSetUserScopesRequest {
            toolkit: toolkit.clone(),
            scopes: to_wire(*pref),
        };
        if let Err(error) = connectors::call::<_, ComposioUserScopesResponse>(
            config,
            methods::SET_USER_SCOPES,
            request,
        )
        .await
        {
            tracing::warn!(%toolkit, %error, "{LOG} legacy pref migration deferred");
            return;
        }
    }
    if !rows.is_empty() {
        let cleared = file_store::update(
            ctx,
            config,
            USER_SCOPES_FILE,
            |prefs: &mut LegacyScopeFile| {
                prefs.clear();
                ((), true)
            },
        )
        .await;
        if let Err(error) = cleared {
            tracing::warn!(%error, "{LOG} legacy pref file could not be emptied");
            return;
        }
        tracing::info!(rows = rows.len(), "{LOG} legacy prefs moved to the module");
    }
    migrated()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(workspace);
}

/// Run the one-time legacy-file migration, so a module-side read of the
/// preference (a visibility query, a filtered listing) sees rows a user stored
/// before the preference moved into the module. Idempotent per workspace; the
/// pref reads and writes below call it themselves.
pub(crate) async fn ensure_migrated(ctx: &crate::core::runtime::DispatchCtx, config: &Config) {
    migrate_legacy(ctx, config).await;
}

/// The scope pref stored for `toolkit` (the module's default when none is).
///
/// **This read fails closed**: an unreachable module or a stored row that
/// cannot be read is an `Err`, never the default. The default is `read+write`,
/// so answering it for a user who turned `write` off would widen what the agent
/// may run. Callers that gate on the pref treat an `Err` as "not allowed".
///
/// # Errors
///
/// An empty toolkit, or the module could not answer.
pub(crate) async fn load(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    toolkit: &str,
) -> Result<UserScopePref, String> {
    let key = UserScopePref::key(toolkit);
    if key.is_empty() {
        return Err("user_scopes: toolkit must not be empty".to_string());
    }
    migrate_legacy(ctx, config).await;
    let request = ComposioGetUserScopesRequest {
        toolkit: key.clone(),
    };
    match connectors::call::<_, ComposioUserScopesResponse>(
        config,
        methods::GET_USER_SCOPES,
        request,
    )
    .await
    {
        Ok(reply) => {
            let pref = from_wire(reply.scopes);
            tracing::debug!(
                toolkit = %key,
                read = pref.read,
                write = pref.write,
                admin = pref.admin,
                "{LOG} pref loaded"
            );
            Ok(pref)
        }
        Err(error) => {
            tracing::warn!(toolkit = %key, %error, "{LOG} pref unavailable, failing closed");
            Err(error)
        }
    }
}

/// Persist the scope pref for `toolkit` in the module.
///
/// **This one fails closed.** A write that reported success without storing
/// anything would leave the user looking at a toggle they just moved while the
/// agent kept the old permissions.
///
/// # Errors
///
/// An empty toolkit, or the module could not store the row.
pub(crate) async fn save(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    toolkit: &str,
    pref: UserScopePref,
) -> Result<(), String> {
    let key = UserScopePref::key(toolkit);
    if key.is_empty() {
        return Err("user_scopes: toolkit must not be empty".to_string());
    }
    migrate_legacy(ctx, config).await;
    let _legacy = LEGACY.lock().await;
    // A migration still deferred would push this toolkit's legacy row again
    // on the next read and undo the choice stored below, so that row is
    // dropped after the write. Drop it only after the write succeeded:
    // dropping it first and then failing the write would fall back to the
    // permissive default.
    let pending = !migrated()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains(&config.workspace_dir);
    // The row the module enforces now, put back if the drop fails, so a save
    // reported as failed leaves the module as the UI will show it.
    let prior = if pending {
        let request = ComposioGetUserScopesRequest {
            toolkit: key.clone(),
        };
        Some(
            connectors::call::<_, ComposioUserScopesResponse>(
                config,
                methods::GET_USER_SCOPES,
                request,
            )
            .await?
            .scopes,
        )
    } else {
        None
    };
    let request = ComposioSetUserScopesRequest {
        toolkit: key.clone(),
        scopes: to_wire(pref),
    };
    connectors::call::<_, ComposioUserScopesResponse>(config, methods::SET_USER_SCOPES, request)
        .await?;
    if let Some(prior) = prior {
        let row = key.clone();
        let cleared = file_store::update(
            ctx,
            config,
            USER_SCOPES_FILE,
            move |prefs: &mut LegacyScopeFile| ((), prefs.remove(&row).is_some()),
        )
        .await;
        if let Err(error) = cleared {
            let request = ComposioSetUserScopesRequest {
                toolkit: key.clone(),
                scopes: prior,
            };
            if let Err(restore) = connectors::call::<_, ComposioUserScopesResponse>(
                config,
                methods::SET_USER_SCOPES,
                request,
            )
            .await
            {
                tracing::error!(toolkit = %key, %restore, "{LOG} prior pref not restored");
            }
            return Err(format!("user_scopes: legacy row not cleared: {error}"));
        }
    }
    tracing::info!(
        toolkit = %key,
        read = pref.read,
        write = pref.write,
        admin = pref.admin,
        "{LOG} pref saved"
    );
    Ok(())
}

#[cfg(test)]
#[path = "scope_prefs_tests.rs"]
mod tests;
