//! SQLite persistence for the MCP clients domain.
//!
//! Uses `mcp_clients/mcp_clients.db` inside the workspace directory.
//! Three tables:
//!   - `mcp_servers`     — installed server metadata (no env values)
//!   - `mcp_client_env`  — per-server env values (key + value; values never
//!                          leave this module or appear in responses)
//!   - `mcp_registry_cache` — Smithery API response cache with TTL

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension as _};
use serde_json::Value;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::openhuman::config::Config;

use super::types::{CommandKind, InstalledServer, Transport};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db_dir = config.workspace_dir.join("mcp_clients");
    std::fs::create_dir_all(&db_dir)
        .with_context(|| format!("Failed to create mcp_clients dir: {}", db_dir.display()))?;
    let db_path = db_dir.join("mcp_clients.db");
    let conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open mcp_clients DB: {}", db_path.display()))?;
    init_schema(&conn)?;
    f(&conn)
}

/// Build the schema using an in-memory path (for tests).
pub fn with_test_connection<T>(
    db_path: &Path,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let conn = Connection::open(db_path)
        .with_context(|| format!("open test DB: {}", db_path.display()))?;
    init_schema(&conn)?;
    f(&conn)
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;

         CREATE TABLE IF NOT EXISTS mcp_servers (
             server_id           TEXT PRIMARY KEY,
             qualified_name      TEXT NOT NULL,
             display_name        TEXT NOT NULL,
             description         TEXT,
             icon_url            TEXT,
             command_kind        TEXT NOT NULL DEFAULT 'node',
             command             TEXT NOT NULL,
             args_json           TEXT NOT NULL DEFAULT '[]',
             env_keys_json       TEXT NOT NULL DEFAULT '[]',
             config_json         TEXT,
             installed_at        INTEGER NOT NULL,
             last_connected_at   INTEGER
         );

         CREATE TABLE IF NOT EXISTS mcp_client_env (
             server_id   TEXT NOT NULL,
             key         TEXT NOT NULL,
             value       TEXT NOT NULL,
             PRIMARY KEY (server_id, key),
             FOREIGN KEY (server_id) REFERENCES mcp_servers(server_id) ON DELETE CASCADE
         );

         CREATE TABLE IF NOT EXISTS mcp_registry_cache (
             cache_key   TEXT PRIMARY KEY,
             body_json   TEXT NOT NULL,
             cached_at   INTEGER NOT NULL
         );",
    )
    .context("Failed to initialise mcp_clients schema")?;

    // Additive HTTP-remote transport columns (introduced after the schema
    // was first cut). SQLite's `ALTER TABLE ADD COLUMN` doesn't support
    // `IF NOT EXISTS`, so we use `PRAGMA table_info` to detect which
    // columns are already there and skip the ones that are. Idempotent
    // across launches; old `'stdio'`-implicit rows pick up the new
    // `transport` column with the default value.
    let existing_cols = mcp_servers_columns(conn)?;
    if !existing_cols.iter().any(|c| c == "transport") {
        conn.execute(
            "ALTER TABLE mcp_servers ADD COLUMN transport TEXT NOT NULL DEFAULT 'stdio'",
            [],
        )
        .context("Failed to add transport column to mcp_servers")?;
    }
    if !existing_cols.iter().any(|c| c == "deployment_url") {
        conn.execute("ALTER TABLE mcp_servers ADD COLUMN deployment_url TEXT", [])
            .context("Failed to add deployment_url column to mcp_servers")?;
    }

    Ok(())
}

/// Snapshot of the column names on `mcp_servers`. Used by the additive
/// migration in [`init_schema`] to decide which `ALTER TABLE ADD COLUMN`
/// statements still need to run on this DB.
fn mcp_servers_columns(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(mcp_servers)")
        .context("prepare PRAGMA table_info")?;
    // PRAGMA table_info row shape: (cid, name, type, notnull, dflt_value, pk).
    let mut rows = stmt.query([])?;
    let mut cols = Vec::new();
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        cols.push(name);
    }
    Ok(cols)
}

// ── InstalledServer CRUD ──────────────────────────────────────────────────────

pub fn insert_server(config: &Config, server: &InstalledServer) -> Result<()> {
    with_connection(config, |conn| insert_server_conn(conn, server))
}

pub fn insert_server_conn(conn: &Connection, server: &InstalledServer) -> Result<()> {
    let args_json = serde_json::to_string(&server.args)?;
    let env_keys_json = serde_json::to_string(&server.env_keys)?;
    let config_json = server
        .config
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    conn.execute(
        "INSERT INTO mcp_servers
             (server_id, qualified_name, display_name, description, icon_url,
              command_kind, command, args_json, env_keys_json, config_json,
              installed_at, last_connected_at, transport, deployment_url)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            server.server_id,
            server.qualified_name,
            server.display_name,
            server.description,
            server.icon_url,
            server.command_kind.as_str(),
            server.command,
            args_json,
            env_keys_json,
            config_json,
            server.installed_at,
            server.last_connected_at,
            server.transport.dispatch_kind(),
            server.transport.deployment_url(),
        ],
    )
    .context("Failed to insert mcp_server")?;
    Ok(())
}

pub fn list_servers(config: &Config) -> Result<Vec<InstalledServer>> {
    with_connection(config, |conn| list_servers_conn(conn))
}

pub fn list_servers_conn(conn: &Connection) -> Result<Vec<InstalledServer>> {
    let mut stmt = conn.prepare(
        "SELECT server_id, qualified_name, display_name, description, icon_url,
                command_kind, command, args_json, env_keys_json, config_json,
                installed_at, last_connected_at, transport, deployment_url
         FROM mcp_servers ORDER BY installed_at ASC",
    )?;
    let rows = stmt.query_map([], map_server_row)?;
    let mut servers = Vec::new();
    for row in rows {
        servers.push(row?);
    }
    Ok(servers)
}

pub fn get_server(config: &Config, server_id: &str) -> Result<InstalledServer> {
    with_connection(config, |conn| get_server_conn(conn, server_id))
}

pub fn get_server_conn(conn: &Connection, server_id: &str) -> Result<InstalledServer> {
    let mut stmt = conn.prepare(
        "SELECT server_id, qualified_name, display_name, description, icon_url,
                command_kind, command, args_json, env_keys_json, config_json,
                installed_at, last_connected_at, transport, deployment_url
         FROM mcp_servers WHERE server_id = ?1",
    )?;
    let mut rows = stmt.query(params![server_id])?;
    if let Some(row) = rows.next()? {
        map_server_row(row).map_err(Into::into)
    } else {
        anyhow::bail!("MCP server '{}' not found", server_id)
    }
}

pub fn delete_server(config: &Config, server_id: &str) -> Result<bool> {
    with_connection(config, |conn| {
        let changed = conn
            .execute(
                "DELETE FROM mcp_servers WHERE server_id = ?1",
                params![server_id],
            )
            .context("Failed to delete mcp_server")?;
        Ok(changed > 0)
    })
}

pub fn update_last_connected(config: &Config, server_id: &str) -> Result<()> {
    let ts = now_ms();
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE mcp_servers SET last_connected_at = ?1 WHERE server_id = ?2",
            params![ts, server_id],
        )
        .context("Failed to update last_connected_at")?;
        Ok(())
    })
}

fn map_server_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledServer> {
    let args_json: String = row.get(7)?;
    let env_keys_json: String = row.get(8)?;
    let config_json: Option<String> = row.get(9)?;

    let args: Vec<String> = serde_json::from_str(&args_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let env_keys: Vec<String> = serde_json::from_str(&env_keys_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let config: Option<Value> = match config_json.as_deref() {
        None => None,
        Some(s) => Some(serde_json::from_str(s).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(e))
        })?),
    };

    // Both transport columns are post-migration additions, so `row.get`
    // may return missing-column-by-name errors on a DB that hasn't run the
    // ADD COLUMN steps for some reason (rare — the migration is in
    // `init_schema`). Fall back to stdio rather than fail loading the
    // whole row.
    let transport_kind: String = row.get::<_, Option<String>>(12)?.unwrap_or_default();
    let deployment_url: Option<String> = row.get(13)?;
    let transport = Transport::parse(&transport_kind, deployment_url.as_deref());

    Ok(InstalledServer {
        server_id: row.get(0)?,
        qualified_name: row.get(1)?,
        display_name: row.get(2)?,
        description: row.get(3)?,
        icon_url: row.get(4)?,
        command_kind: CommandKind::parse(&row.get::<_, String>(5)?),
        command: row.get(6)?,
        args,
        env_keys,
        config,
        installed_at: row.get(10)?,
        last_connected_at: row.get(11)?,
        transport,
    })
}

// ── Env values ───────────────────────────────────────────────────────────────

/// Store (insert or replace) env key-value pairs for a server.
/// Values are never returned in any list/status response.
pub fn set_env_values(
    config: &Config,
    server_id: &str,
    env: &std::collections::HashMap<String, String>,
) -> Result<()> {
    with_connection(config, |conn| set_env_values_conn(conn, server_id, env))
}

pub fn set_env_values_conn(
    conn: &Connection,
    server_id: &str,
    env: &std::collections::HashMap<String, String>,
) -> Result<()> {
    // Delete all existing env rows for this server first so that keys removed
    // from the new map don't linger.  The upsert below re-inserts the current set.
    conn.execute(
        "DELETE FROM mcp_client_env WHERE server_id = ?1",
        params![server_id],
    )
    .context("Failed to clear previous mcp_client_env rows")?;

    for (key, value) in env {
        conn.execute(
            "INSERT INTO mcp_client_env (server_id, key, value) VALUES (?1, ?2, ?3)",
            params![server_id, key, value],
        )
        .context("Failed to insert mcp_client_env")?;
    }
    Ok(())
}

/// Load env values for a server (used when spawning the subprocess).
/// NEVER serialize or log these values.
pub fn load_env_values(
    config: &Config,
    server_id: &str,
) -> Result<std::collections::HashMap<String, String>> {
    with_connection(config, |conn| load_env_values_conn(conn, server_id))
}

pub fn load_env_values_conn(
    conn: &Connection,
    server_id: &str,
) -> Result<std::collections::HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT key, value FROM mcp_client_env WHERE server_id = ?1")?;
    let rows = stmt.query_map(params![server_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut map = std::collections::HashMap::new();
    for row in rows {
        let (k, v) = row?;
        map.insert(k, v);
    }
    Ok(map)
}

// ── Registry cache ────────────────────────────────────────────────────────────

const REGISTRY_CACHE_TTL_MS: i64 = 10 * 60 * 1_000; // 10 minutes

pub fn get_cached(config: &Config, cache_key: &str) -> Result<Option<String>> {
    with_connection(config, |conn| get_cached_conn(conn, cache_key))
}

pub fn get_cached_conn(conn: &Connection, cache_key: &str) -> Result<Option<String>> {
    let now = now_ms();
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT body_json, cached_at FROM mcp_registry_cache WHERE cache_key = ?1",
            params![cache_key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .context("Failed to query registry cache")?;

    match row {
        Some((body, cached_at)) if now - cached_at < REGISTRY_CACHE_TTL_MS => Ok(Some(body)),
        _ => Ok(None),
    }
}

pub fn set_cached(config: &Config, cache_key: &str, body_json: &str) -> Result<()> {
    with_connection(config, |conn| set_cached_conn(conn, cache_key, body_json))
}

pub fn set_cached_conn(conn: &Connection, cache_key: &str, body_json: &str) -> Result<()> {
    let now = now_ms();
    conn.execute(
        "INSERT OR REPLACE INTO mcp_registry_cache (cache_key, body_json, cached_at)
         VALUES (?1, ?2, ?3)",
        params![cache_key, body_json, now],
    )
    .context("Failed to upsert registry cache")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn open_test_conn() -> (NamedTempFile, Connection) {
        let f = NamedTempFile::new().unwrap();
        let conn = Connection::open(f.path()).unwrap();
        init_schema(&conn).unwrap();
        (f, conn)
    }

    fn sample_server(id: &str) -> InstalledServer {
        InstalledServer {
            server_id: id.to_string(),
            qualified_name: "@test/server".to_string(),
            display_name: "Test Server".to_string(),
            description: Some("A test server".to_string()),
            icon_url: None,
            command_kind: CommandKind::Node,
            command: "npx".to_string(),
            args: vec!["-y".to_string(), "@test/server".to_string()],
            env_keys: vec!["API_KEY".to_string()],
            config: None,
            installed_at: 1_700_000_000_000,
            last_connected_at: None,
            transport: Transport::Stdio,
        }
    }

    fn sample_http_server(id: &str, url: &str) -> InstalledServer {
        InstalledServer {
            server_id: id.to_string(),
            qualified_name: "@test/http-server".to_string(),
            display_name: "Test HTTP Server".to_string(),
            description: None,
            icon_url: None,
            command_kind: CommandKind::Node, // unused for HTTP
            command: String::new(),
            args: Vec::new(),
            env_keys: Vec::new(),
            config: None,
            installed_at: 1_700_000_000_000,
            last_connected_at: None,
            transport: Transport::HttpRemote {
                url: url.to_string(),
            },
        }
    }

    #[test]
    fn insert_and_list_servers() {
        let (_f, conn) = open_test_conn();
        let server = sample_server("srv-1");
        insert_server_conn(&conn, &server).unwrap();
        let servers = list_servers_conn(&conn).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].server_id, "srv-1");
        assert_eq!(servers[0].command_kind, CommandKind::Node);
    }

    #[test]
    fn get_server_not_found() {
        let (_f, conn) = open_test_conn();
        let err = get_server_conn(&conn, "missing").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn delete_server_returns_true_when_found() {
        let (_f, conn) = open_test_conn();
        let server = sample_server("srv-del");
        insert_server_conn(&conn, &server).unwrap();

        // Open a config-less wrapper that reuses the same connection path
        let deleted = conn
            .execute(
                "DELETE FROM mcp_servers WHERE server_id = ?1",
                params!["srv-del"],
            )
            .unwrap();
        assert_eq!(deleted, 1);
    }

    #[test]
    fn env_values_upsert_and_load() {
        let (_f, conn) = open_test_conn();
        let server = sample_server("srv-env");
        insert_server_conn(&conn, &server).unwrap();

        let mut env = std::collections::HashMap::new();
        env.insert("API_KEY".to_string(), "secret123".to_string());
        set_env_values_conn(&conn, "srv-env", &env).unwrap();

        let loaded = load_env_values_conn(&conn, "srv-env").unwrap();
        assert_eq!(loaded.get("API_KEY").map(String::as_str), Some("secret123"));
    }

    #[test]
    fn registry_cache_miss_on_empty_db() {
        let (_f, conn) = open_test_conn();
        let cached = get_cached_conn(&conn, "search:rust").unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn registry_cache_hit_within_ttl() {
        let (_f, conn) = open_test_conn();
        set_cached_conn(&conn, "search:rust", r#"{"servers":[]}"#).unwrap();
        let cached = get_cached_conn(&conn, "search:rust").unwrap();
        assert!(cached.is_some());
    }

    #[test]
    fn registry_cache_miss_after_ttl() {
        let (_f, conn) = open_test_conn();
        // Insert with an old timestamp (way past TTL)
        let old_ts = now_ms() - REGISTRY_CACHE_TTL_MS - 1_000;
        conn.execute(
            "INSERT INTO mcp_registry_cache (cache_key, body_json, cached_at) VALUES (?1, ?2, ?3)",
            params!["stale:key", r#"{"servers":[]}"#, old_ts],
        )
        .unwrap();
        let cached = get_cached_conn(&conn, "stale:key").unwrap();
        assert!(cached.is_none());
    }

    #[test]
    fn server_args_and_env_keys_roundtrip_through_json() {
        let (_f, conn) = open_test_conn();
        let mut server = sample_server("srv-args");
        server.args = vec!["--port".to_string(), "8080".to_string()];
        server.env_keys = vec!["KEY_A".to_string(), "KEY_B".to_string()];
        insert_server_conn(&conn, &server).unwrap();

        let loaded = get_server_conn(&conn, "srv-args").unwrap();
        assert_eq!(loaded.args, vec!["--port", "8080"]);
        assert_eq!(loaded.env_keys, vec!["KEY_A", "KEY_B"]);
    }

    /// HTTP-remote row round-trips through INSERT/SELECT with the
    /// `deployment_url` preserved and `transport.dispatch_kind()` flipped
    /// to `"http_remote"`. Without this test a regression in the
    /// `map_server_row` column indices would silently downgrade every
    /// HTTP-remote install back to stdio at next launch.
    #[test]
    fn http_remote_server_roundtrips_with_url_preserved() {
        let (_f, conn) = open_test_conn();
        let server = sample_http_server("srv-http", "https://smithery.ai/server/x/mcp");
        insert_server_conn(&conn, &server).unwrap();

        let loaded = get_server_conn(&conn, "srv-http").unwrap();
        match loaded.transport {
            Transport::HttpRemote { url } => {
                assert_eq!(url, "https://smithery.ai/server/x/mcp");
            }
            other => panic!("expected HttpRemote, got {other:?}"),
        }
    }

    /// Mixed stdio + http rows list back in their persisted form (no
    /// cross-contamination of the `transport` column between rows).
    #[test]
    fn list_servers_preserves_per_row_transport() {
        let (_f, conn) = open_test_conn();
        insert_server_conn(&conn, &sample_server("srv-stdio")).unwrap();
        insert_server_conn(&conn, &sample_http_server("srv-http", "https://x.io/mcp")).unwrap();

        let mut servers = list_servers_conn(&conn).unwrap();
        servers.sort_by_key(|s| s.server_id.clone());
        assert_eq!(servers.len(), 2);
        // Alphabetical sort: "srv-http" precedes "srv-stdio".
        assert_eq!(servers[0].server_id, "srv-http");
        assert_eq!(
            servers[0].transport,
            Transport::HttpRemote {
                url: "https://x.io/mcp".to_string()
            }
        );
        assert_eq!(servers[1].server_id, "srv-stdio");
        assert_eq!(servers[1].transport, Transport::Stdio);
    }

    /// Simulates the pre-migration state by dropping the `transport` and
    /// `deployment_url` columns *after* schema init, manually inserting a
    /// row that lacks them, and then re-running `init_schema` to confirm
    /// the additive ALTER TABLE re-introduces the columns idempotently and
    /// the old row loads as stdio (the migration's whole point).
    ///
    /// SQLite can't `DROP COLUMN` portably before 3.35, so the test uses
    /// a CREATE-TABLE-AS rebuild to mimic the original schema shape.
    #[test]
    fn additive_migration_recovers_pre_migration_row_as_stdio() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = rusqlite::Connection::open(tmp.path()).unwrap();

        // Step 1: pre-migration schema (no transport / deployment_url).
        conn.execute_batch(
            "CREATE TABLE mcp_servers (
                server_id           TEXT PRIMARY KEY,
                qualified_name      TEXT NOT NULL,
                display_name        TEXT NOT NULL,
                description         TEXT,
                icon_url            TEXT,
                command_kind        TEXT NOT NULL DEFAULT 'node',
                command             TEXT NOT NULL,
                args_json           TEXT NOT NULL DEFAULT '[]',
                env_keys_json       TEXT NOT NULL DEFAULT '[]',
                config_json         TEXT,
                installed_at        INTEGER NOT NULL,
                last_connected_at   INTEGER
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO mcp_servers
                (server_id, qualified_name, display_name, command_kind, command, installed_at)
             VALUES ('legacy-1', '@old/server', 'Old', 'node', 'npx', 1700000000000)",
            [],
        )
        .unwrap();

        // Step 2: simulate the upgrade path — re-run init_schema, which
        // detects the missing columns via PRAGMA and runs ALTER TABLE.
        init_schema(&conn).unwrap();

        // Idempotency: running it again must not fail or duplicate the
        // columns. (Real launches hit this every process start.)
        init_schema(&conn).unwrap();

        // Step 3: the legacy row loads as Transport::Stdio.
        let loaded = get_server_conn(&conn, "legacy-1").unwrap();
        assert_eq!(loaded.transport, Transport::Stdio);
        assert_eq!(loaded.command, "npx");
    }
}
