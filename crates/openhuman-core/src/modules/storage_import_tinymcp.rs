//! One-time, read-only import of tinymcp's legacy SQLite files into the
//! module's storage.
//!
//! Before the storage contract, tinymcp kept installed servers, their
//! environment values and the write-audit log in two files under a data
//! directory (`mcp_clients/mcp_clients.db`, `mcp_audit/mcp_audit.db`). The
//! module now keeps them in its own scope of the host's storage
//! (`module:tinymcp@<base>`): servers and name claims as documents, each
//! server's environment as a secret, the audit log as a stream. It no longer
//! reads the files, so an install upgraded with servers still configured must
//! not lose them. The host owns this import (tinymcp `docs/specs/storage-ports.md`).
//!
//! * **Legacy files are never modified.** The files (with `-wal` / `-shm`) are
//!   copied to a scratch directory and the copy is read.
//! * **Nothing is overwritten.** Documents use `Precondition::Absent` and a
//!   secret that already exists is kept, so records the module wrote since are
//!   never replaced.
//! * **Runs once per data directory.** A marker document in a collection the
//!   module's grant does not include records it; a server deleted afterwards
//!   is not resurrected from the untouched file. A file that cannot be read
//!   writes no marker and is retried at the next start.
//! * **What is not carried over.** The registry browse cache, the catalog
//!   index and the tool cache are re-derivable from upstream, and the tool
//!   cache's fingerprints are keyed with a fresh per-install key.
//!
//! The audit log is imported only into an empty stream, in one batch, so a
//! retry after a failed attempt cannot duplicate it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use sha2::Digest;
use tinystoragedrivers::secrets::{DocumentSecrets, SecretStore};
use tinystoragedrivers::{CollectionSpec, DocumentStore, ErrorKind, Precondition, ScopedStorage};

use crate::core::runtime::DispatchCtx;
use crate::storage::rusqlite::{self, Connection, OpenFlags};

const LOG_PREFIX: &str = "[mcp-import]";
const MODULE_ID: &str = "tinymcp";
const CLIENTS_DB: &str = "mcp_clients/mcp_clients.db";
const AUDIT_DB: &str = "mcp_audit/mcp_audit.db";

/// The module's own names (restated in `storage_grant_tinymcp`).
const SERVERS: &str = "mcp_servers";
const SERVER_NAMES: &str = "mcp_server_names";
const ENV_SECRET_PREFIX: &str = "mcp_client_env/";
const AUDIT_STREAM: &str = "mcp_audit";
/// Host-only: not in the module's grant, so the module cannot see it.
const MARKER_COLLECTION: &str = "mcp_host_import";

/// How one import attempt ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// No storage backend is installed; the module is not using this storage.
    NoBackend,
    /// The marker says this data directory was already imported.
    AlreadyDone,
    /// Neither legacy file exists.
    NoSource,
    /// The files were read and their records written.
    Imported {
        servers: usize,
        secrets: usize,
        audit: usize,
    },
}

/// Imports the legacy files under `data_dir` into the module's scope of the
/// installed backend.
pub(crate) async fn import_legacy(ctx: &DispatchCtx, data_dir: &Path) -> Result<Outcome> {
    let Some(backend) = crate::storage::installed() else {
        return Ok(Outcome::NoBackend);
    };
    let scope = super::storage_host::current_module_scope(ctx, MODULE_ID)
        .map_err(|error| anyhow!("resolve the module scope: {:?}", error.kind()))?;
    let scoped = backend
        .for_scope(&scope)
        .map_err(|error| anyhow!("open the module scope: {:?}", error.kind()))?;
    let keys = crate::storage::secrets::keys()
        .map_err(|error| anyhow!("no key provider for module secrets: {:?}", error.kind()))?;
    let secrets = DocumentSecrets::new(&scoped, keys);
    import_into(ctx, &scoped, &secrets, data_dir).await
}

async fn import_into(
    ctx: &DispatchCtx,
    scoped: &ScopedStorage,
    secrets: &dyn SecretStore,
    data_dir: &Path,
) -> Result<Outcome> {
    let docs = scoped.documents();
    let marker = marker_id(data_dir);
    if crate::storage::import_state::host_import_done(scoped, MARKER_COLLECTION, &marker)
        .await
        .map_err(|error| anyhow!("read the import state: {:?}", error.kind()))?
    {
        return Ok(Outcome::AlreadyDone);
    }
    let clients = data_dir.join(CLIENTS_DB);
    let audit = data_dir.join(AUDIT_DB);
    if !clients.exists() && !audit.exists() {
        return Ok(Outcome::NoSource);
    }

    // The files are read on a blocking thread, from copies.
    let (legacy_servers, legacy_audit) = ctx
        .spawn_blocking(move || -> Result<_> {
            let servers = if clients.exists() {
                read_servers(&clients)?
            } else {
                Vec::new()
            };
            let audit = if audit.exists() {
                read_audit(&audit)?
            } else {
                Vec::new()
            };
            Ok((servers, audit))
        })
        .await
        .context("the legacy reader panicked")??;

    for name in [SERVERS, SERVER_NAMES] {
        docs.ensure_collection(&CollectionSpec::new(name))
            .await
            .map_err(|error| anyhow!("declare {name}: {:?}", error.kind()))?;
    }

    let (mut servers, mut secret_count) = (0usize, 0usize);
    for server in legacy_servers {
        if put_absent(docs, SERVERS, &server.id, server.doc).await? {
            servers += 1;
        }
        put_absent(docs, SERVER_NAMES, &server.claim_id, server.claim).await?;
        if !server.env.is_empty() {
            let name = format!("{ENV_SECRET_PREFIX}{}", server.id);
            let present = secrets
                .get(&name)
                .await
                .map_err(|error| anyhow!("read a secret: {:?}", error.kind()))?
                .is_some();
            if !present {
                let bytes = zeroize::Zeroizing::new(serde_json::to_vec(&server.env)?);
                secrets
                    .set(&name, &bytes)
                    .await
                    .map_err(|error| anyhow!("write a secret: {:?}", error.kind()))?;
                secret_count += 1;
            }
        }
    }

    let mut audited = 0usize;
    if !legacy_audit.is_empty() {
        let streams = scoped.streams();
        let held = streams
            .len(AUDIT_STREAM)
            .await
            .map_err(|error| anyhow!("read the audit stream: {:?}", error.kind()))?;
        if held == 0 {
            audited = legacy_audit.len();
            streams
                .append_batch(AUDIT_STREAM, legacy_audit)
                .await
                .map_err(|error| anyhow!("append the audit log: {:?}", error.kind()))?;
        }
    }

    crate::storage::import_state::host_import_mark(scoped, MARKER_COLLECTION, &marker)
        .await
        .map_err(|error| anyhow!("write the import state: {:?}", error.kind()))?;
    tracing::info!(
        "{LOG_PREFIX} imported servers={servers} secrets={secret_count} audit={audited}"
    );
    Ok(Outcome::Imported {
        servers,
        secrets: secret_count,
        audit: audited,
    })
}

async fn put_absent(
    docs: &Arc<dyn DocumentStore>,
    collection: &str,
    id: &str,
    doc: Value,
) -> Result<bool> {
    match docs.put(collection, id, doc, Precondition::Absent).await {
        Ok(_) => Ok(true),
        Err(error) if matches!(error.kind(), ErrorKind::Conflict | ErrorKind::AlreadyExists) => {
            Ok(false)
        }
        Err(error) => Err(anyhow!("write {collection}: {:?}", error.kind())),
    }
}

/// One legacy server, in the shapes the module stores.
struct LegacyServer {
    id: String,
    doc: Value,
    claim_id: String,
    claim: Value,
    env: std::collections::BTreeMap<String, String>,
}

/// Copies `path` (and its WAL companions) next to nothing the host owns and
/// opens the copy read-only.
fn open_copy(path: &Path, scratch: &Path) -> Result<Connection> {
    let name = path.file_name().context("legacy file has no name")?;
    let copy = scratch.join(name);
    std::fs::copy(path, &copy).with_context(|| format!("copy {}", path.display()))?;
    for suffix in ["-wal", "-shm"] {
        let mut companion = path.as_os_str().to_owned();
        companion.push(suffix);
        let companion = PathBuf::from(companion);
        if companion.exists() {
            let mut target = copy.as_os_str().to_owned();
            target.push(suffix);
            std::fs::copy(&companion, PathBuf::from(target))
                .with_context(|| format!("copy {}", companion.display()))?;
        }
    }
    Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| format!("open the copy of {}", path.display()))
}

fn table_exists(conn: &Connection, table: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
}

fn columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut statement = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names)
}

fn json_or<T>(text: Option<String>, fallback: T) -> T
where
    T: serde::de::DeserializeOwned,
{
    text.and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(fallback)
}

fn read_servers(path: &Path) -> Result<Vec<LegacyServer>> {
    let scratch = tempfile::tempdir().context("scratch directory")?;
    let conn = open_copy(path, scratch.path())?;
    if !table_exists(&conn, "mcp_servers")? {
        return Ok(Vec::new());
    }
    let present = columns(&conn, "mcp_servers")?;
    // Columns added after the first schema read as what every older row was.
    let optional = |column: &str, fallback: &str| {
        if present.iter().any(|name| name == column) {
            column.to_string()
        } else {
            fallback.to_string()
        }
    };
    let sql = format!(
        "SELECT server_id, qualified_name, display_name, description, icon_url, \
                command_kind, command, args_json, env_keys_json, config_json, \
                installed_at, last_connected_at, {}, {}, {}, {} \
         FROM mcp_servers ORDER BY rowid",
        optional("transport", "'stdio'"),
        optional("deployment_url", "NULL"),
        optional("enabled", "1"),
        optional("catalog_managed", "0"),
    );
    let mut env = std::collections::BTreeMap::<String, std::collections::BTreeMap<_, _>>::new();
    if table_exists(&conn, "mcp_client_env")? {
        let mut statement =
            conn.prepare("SELECT server_id, key, value FROM mcp_client_env ORDER BY rowid")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (server, key, value) = row?;
            env.entry(server).or_default().insert(key, value);
        }
    }
    let mut statement = conn.prepare(&sql)?;
    let rows = statement.query_map([], |row| {
        let id: String = row.get(0)?;
        let qualified_name: String = row.get(1)?;
        let installed_at: i64 = row.get(10)?;
        let mut doc = json!({
            "server_id": id,
            "qualified_name": qualified_name,
            "display_name": row.get::<_, String>(2)?,
            "command_kind": row.get::<_, String>(5)?,
            "command": row.get::<_, String>(6)?,
            "args": json_or(row.get::<_, Option<String>>(7)?, Vec::<String>::new()),
            "env_keys": json_or(row.get::<_, Option<String>>(8)?, Vec::<String>::new()),
            "installed_at": installed_at,
            "transport": row.get::<_, String>(12)?,
            "enabled": row.get::<_, i64>(14)? != 0,
            "catalog_managed": row.get::<_, i64>(15)? != 0,
        });
        let map = doc.as_object_mut().expect("an object");
        let text = |index: usize| row.get::<_, Option<String>>(index);
        if let Some(description) = text(3)? {
            map.insert("description".into(), json!(description));
        }
        if let Some(icon) = text(4)? {
            map.insert("icon_url".into(), json!(icon));
        }
        if let Some(config) = text(9)?.and_then(|text| serde_json::from_str::<Value>(&text).ok()) {
            map.insert("config".into(), config);
        }
        if let Some(at) = row.get::<_, Option<i64>>(11)? {
            map.insert("last_connected_at".into(), json!(at));
        }
        if let Some(url) = text(13)? {
            map.insert("deployment_url".into(), json!(url));
        }
        Ok((id, qualified_name, installed_at, doc))
    })?;
    let mut servers = Vec::new();
    for row in rows {
        let (id, qualified_name, installed_at, doc) = row?;
        let claim_id = hex::encode(sha2::Sha256::digest(qualified_name.as_bytes()));
        let claim = json!({
            "qualified_name": qualified_name,
            "server_id": id,
            "claimed_at": installed_at,
        });
        servers.push(LegacyServer {
            env: env.remove(&id).unwrap_or_default(),
            id,
            doc,
            claim_id,
            claim,
        });
    }
    Ok(servers)
}

fn read_audit(path: &Path) -> Result<Vec<Value>> {
    let scratch = tempfile::tempdir().context("scratch directory")?;
    let conn = open_copy(path, scratch.path())?;
    if !table_exists(&conn, "mcp_writes")? {
        return Ok(Vec::new());
    }
    let mut statement = conn.prepare(
        "SELECT timestamp_ms, client_info, tool_name, args_summary, resulting_chunk_id, \
                success, error_message \
         FROM mcp_writes ORDER BY id",
    )?;
    let rows = statement.query_map([], |row| {
        let mut entry = json!({
            "timestamp_ms": row.get::<_, i64>(0)?,
            "client_info": row.get::<_, String>(1)?,
            "tool_name": row.get::<_, String>(2)?,
            "args_summary": json_or(row.get::<_, Option<String>>(3)?, Value::Null),
            "success": row.get::<_, i64>(5)? != 0,
        });
        let map = entry.as_object_mut().expect("an object");
        if let Some(chunk) = row.get::<_, Option<String>>(4)? {
            map.insert("resulting_chunk_id".into(), json!(chunk));
        }
        if let Some(error) = row.get::<_, Option<String>>(6)? {
            map.insert("error_message".into(), json!(error));
        }
        Ok(entry)
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn marker_id(data_dir: &Path) -> String {
    let digest = sha2::Sha256::digest(data_dir.as_os_str().as_encoded_bytes());
    format!("mcp@{}", &hex::encode(digest)[..16])
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
#[path = "storage_import_tinymcp_tests.rs"]
mod tests;
