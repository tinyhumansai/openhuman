//! Composio host state that is not memory: connected-account identities and
//! per-toolkit scope prefs.
//!
//! Each named value is one serde value in the `composio_state` collection of
//! the storage backend ([`super::file_store_documents`]), under the acting
//! agent's scope: the installed backend, or the workspace default when the
//! host installed none ([`crate::storage::local`]). A read of a missing value
//! is the default value; every read-modify-write is a compare-and-swap loop,
//! so two cores that share the scope never lose each other's change.

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::config::Config;

/// Connected-account identities value name.
pub(crate) const IDENTITIES_FILE: &str = "composio_identities.json";

/// Per-toolkit agent scope prefs value name.
pub(crate) const USER_SCOPES_FILE: &str = "composio_user_scopes.json";

/// Moves a pre-documents `<workspace>/integrations/<name>` file into the
/// document store, once: the value is stored only if none exists yet, then a
/// marker document records that the import finished. The legacy file is only
/// ever read, never renamed or rewritten, so the old data stays byte-identical.
/// A missing file is the common case and a no-op.
async fn import_legacy(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    docs: &super::file_store_documents::Docs,
    name: &'static str,
) -> Result<(), String> {
    let marker_docs = docs.clone();
    let done = ctx
        .spawn_blocking(move || marker_docs.legacy_imported(name))
        .await
        .map_err(|error| format!("[composio:store] checking legacy {name} failed: {error}"))?
        .map_err(|error| format!("[composio:store] checking legacy {name} failed: {error:#}"))?;
    if done {
        return Ok(());
    }
    let path = config.workspace_dir.join("integrations").join(name);
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "[composio:store] reading legacy {name} failed: {error}"
            ))
        }
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("[composio:store] parsing legacy {name} failed: {error}"))?;
    let docs = docs.clone();
    let wrote = ctx
        .spawn_blocking(move || docs.import_if_absent(name, value))
        .await
        .map_err(|error| format!("[composio:store] importing legacy {name} failed: {error}"))?
        .map_err(|error| format!("[composio:store] importing legacy {name} failed: {error:#}"))?;
    log::debug!("[composio:store] legacy import name={name} wrote={wrote}");
    Ok(())
}

/// The value stored under `name`, or `T::default()` when there is none.
///
/// # Errors
///
/// The value cannot be read or does not parse as `T`.
pub(crate) async fn load<T>(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    name: &'static str,
) -> Result<T, String>
where
    T: DeserializeOwned + Default + Send + 'static,
{
    let docs = super::file_store_documents::current(&config.workspace_dir)
        .map_err(|e| format!("[composio:store] {e:#}"))?;
    import_legacy(ctx, config, &docs, name).await?;
    // The document port is reached through a blocking bridge; keep it off the
    // async worker, under the caller's context.
    ctx.spawn_blocking(move || docs.load(name))
        .await
        .map_err(|error| format!("[composio:store] reading {name} failed: {error}"))?
        .map_err(|error| format!("[composio:store] reading {name} failed: {error:#}"))
}

/// Applies `change` to the value under `name` and writes it back when
/// `change` reports a change, returning its result. `change` is retried on a
/// conflict and must be repeatable.
///
/// # Errors
///
/// The value cannot be read or written.
pub(crate) async fn update<T, R>(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    name: &'static str,
    change: impl Fn(&mut T) -> (R, bool) + Send + Sync + 'static,
) -> Result<R, String>
where
    T: DeserializeOwned + Serialize + Default + Send + 'static,
    R: Send + 'static,
{
    let docs = super::file_store_documents::current(&config.workspace_dir)
        .map_err(|e| format!("[composio:store] {e:#}"))?;
    import_legacy(ctx, config, &docs, name).await?;
    ctx.spawn_blocking(move || docs.update(name, change))
        .await
        .map_err(|error| format!("[composio:store] updating {name} failed: {error}"))?
        .map_err(|error| format!("[composio:store] updating {name} failed: {error:#}"))
}

/// Replaces the value under `name` with `value`.
///
/// # Errors
///
/// The value cannot be serialized or written.
#[cfg(test)]
pub(crate) async fn save<T>(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    name: &'static str,
    value: &T,
) -> Result<(), String>
where
    T: Serialize,
{
    let docs = super::file_store_documents::current(&config.workspace_dir)
        .map_err(|e| format!("[composio:store] {e:#}"))?;
    let value = serde_json::to_value(value)
        .map_err(|error| format!("[composio:store] serializing failed: {error}"))?;
    ctx.spawn_blocking(move || docs.save(name, &value))
        .await
        .map_err(|error| format!("[composio:store] writing {name} failed: {error}"))?
        .map_err(|error| format!("[composio:store] writing {name} failed: {error:#}"))
}

#[cfg(test)]
#[path = "file_store_tests.rs"]
mod tests;
