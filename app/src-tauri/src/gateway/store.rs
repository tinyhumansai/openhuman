//! Where gateway records live.
//!
//! Shell-side JSON beside the other host-owned state, deliberately **not** the
//! renderer's `localStorage`. An SSH identity path and a remote bearer are
//! materially more sensitive than a window position, and the renderer's own
//! notes on the cloud token (`app/src/utils/configPersistence.ts`, audit U3)
//! already say that a renderer XSS can read anything kept there. Putting a
//! second, longer-lived credential beside it would widen an exposure the app
//! is already trying to close.
//!
//! The frontend therefore holds a gateway *id* and asks the shell for the rest.

use std::io;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::types::{validate_remote_transport, Gateway, GatewaySpec, DESKTOP_ID};

/// The file gateway records are written to.
const STORE_FILE: &str = "gateways.json";

/// The on-disk shape.
///
/// A struct rather than a bare `Vec` so a later field — a default gateway, a
/// schema version — is an additive change instead of a migration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredGateways {
    #[serde(default)]
    gateways: Vec<Gateway>,
}

fn store_path() -> PathBuf {
    crate::file_logging::resolve_data_dir().join(STORE_FILE)
}

/// Every configured gateway, with the desktop one always first.
///
/// A read failure — missing file, unparseable JSON — yields just the desktop
/// gateway rather than an error. The core in this process is always reachable,
/// so there is always a correct answer, and refusing to list anything would
/// strand the user with no way back to a working gateway.
#[must_use]
pub fn list() -> Vec<Gateway> {
    let mut gateways = vec![Gateway::desktop()];
    gateways.extend(read().gateways.into_iter().filter(|gateway| {
        // A stored record claiming the reserved id would shadow the one
        // guaranteed to work.
        gateway.id != DESKTOP_ID
    }));
    gateways
}

/// Look one up by id.
#[must_use]
pub fn get(id: &str) -> Option<Gateway> {
    list().into_iter().find(|gateway| gateway.id == id)
}

/// Add or replace a gateway.
///
/// # Errors
///
/// Returns a message when the id is reserved, or when the record cannot be
/// written.
pub fn save(gateway: Gateway) -> Result<(), String> {
    if gateway.id == DESKTOP_ID {
        return Err("the desktop gateway is built in and cannot be replaced".to_owned());
    }
    if gateway.id.trim().is_empty() {
        return Err("a gateway needs an id".to_owned());
    }
    if let GatewaySpec::Remote { url, token } = &gateway.spec {
        validate_remote_transport(url, token.as_deref())?;
    }

    let mut stored = read_checked()?;
    match stored
        .gateways
        .iter_mut()
        .find(|existing| existing.id == gateway.id)
    {
        Some(existing) => *existing = gateway,
        None => stored.gateways.push(gateway),
    }
    write(&stored)
}

/// Forget a gateway.
///
/// # Errors
///
/// Returns a message when the id is reserved, or when the record cannot be
/// written. Removing an id that was never stored succeeds: the caller wanted it
/// gone, and it is.
pub fn delete(id: &str) -> Result<(), String> {
    if id == DESKTOP_ID {
        return Err("the desktop gateway is built in and cannot be removed".to_owned());
    }
    let mut stored = read_checked()?;
    stored.gateways.retain(|gateway| gateway.id != id);
    write(&stored)
}

fn read() -> StoredGateways {
    read_checked().unwrap_or_default()
}

/// Read the stored gateways, failing loudly instead of silently discarding the
/// user's records.
///
/// Returns the stored set when the file parses, `Ok(default)` when no file has
/// been written yet, and a distinct error when an existing file is unreadable
/// or malformed. `save`/`delete` use this so a transient read failure or a
/// corrupt file aborts before `write` can clobber every prior record with an
/// empty state; the tolerant `read`/`list` fallback is reserved for display
/// paths where the core in this process is still the correct answer.
fn read_checked() -> Result<StoredGateways, String> {
    let path = store_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).map_err(|error| {
            // Loud, because a record the user configured has stopped being
            // visible and they are about to wonder why.
            format!(
                "{STORE_FILE} is not readable as gateway records ({error}); refusing to overwrite it"
            )
        }),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(StoredGateways::default()),
        Err(error) => {
            log::warn!("[gateway][store] read {STORE_FILE} failed: {error}");
            Err(format!("could not read {STORE_FILE}: {error}"))
        }
    }
}

fn write(stored: &StoredGateways) -> Result<(), String> {
    let path = store_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {STORE_FILE} directory: {error}"))?;
    }
    let text = serde_json::to_string_pretty(stored)
        .map_err(|error| format!("could not serialize gateway records: {error}"))?;

    // The file can hold an SSH identity path and a remote bearer token, so it
    // must not be world-readable. Write to a temp file in the same directory
    // so a crash mid-write cannot leave a truncated `gateways.json` (the reader
    // would then discard every saved gateway), then rename it over the real
    // path once the bytes are on disk. On Unix, create the temp file
    // owner-only and repair an existing file's permissions too.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let temp_path = path.with_extension("json.tmp");
    let write_result = (|| -> io::Result<()> {
        let mut file = options.open(&temp_path)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp_path, &path)?;
        Ok(())
    })();
    if let Err(error) = write_result {
        // Don't leave a half-written temp file behind after a failure.
        let _ = std::fs::remove_file(&temp_path);
        return Err(format!("could not write {STORE_FILE}: {error}"));
    }

    #[cfg(unix)]
    {
        if let Ok(metadata) = std::fs::metadata(&path) {
            if metadata.permissions().mode() & 0o077 != 0 {
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }

    log::debug!(
        "[gateway][store] wrote {} record(s) to {STORE_FILE}",
        stored.gateways.len()
    );
    Ok(())
}
