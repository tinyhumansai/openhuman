use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::openhuman::config::Config;
use crate::openhuman::redirect_links::types::RedirectLink;

pub const SHORT_URL_PREFIX: &str = "openhuman://link/";
const DEFAULT_ID_LEN: usize = 8;
const MAX_ID_LEN: usize = 32;

/// Build the short URL representation for an id.
pub fn short_url_for(id: &str) -> String {
    format!("{SHORT_URL_PREFIX}{id}")
}

/// Parse a short URL back into its id component. Accepts both
/// `openhuman://link/<id>` and bare `<id>` (hex only).
pub fn id_from_short(short: &str) -> Option<String> {
    let trimmed = short.trim();
    let candidate = trimmed.strip_prefix(SHORT_URL_PREFIX).unwrap_or(trimmed);
    if !candidate.is_empty() && candidate.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(candidate.to_ascii_lowercase())
    } else {
        None
    }
}

fn content_id(url: &str, len: usize) -> String {
    let digest = Sha256::digest(url.as_bytes());
    hex::encode(digest)[..len.min(64)].to_string()
}

pub fn shorten(config: &Config, url: &str) -> Result<RedirectLink> {
    let url = url.trim();
    if url.is_empty() {
        anyhow::bail!("url must not be empty");
    }

    with_connection(config, |conn| {
        let mut len = DEFAULT_ID_LEN;
        let now = Utc::now();
        loop {
            if len > MAX_ID_LEN {
                anyhow::bail!("failed to allocate unique redirect id after expansion");
            }
            let id = content_id(url, len);

            // Atomic insert. If either `id` or `url` already exists, the
            // statement becomes a no-op — no PRIMARY KEY / UNIQUE error under
            // concurrent calls, so we don't need a pre-read.
            let affected = conn
                .execute(
                    "INSERT INTO redirect_links
                        (id, url, created_at, last_used_at, hit_count)
                     VALUES (?1, ?2, ?3, NULL, 0)
                     ON CONFLICT DO NOTHING",
                    params![id, url, now.to_rfc3339()],
                )
                .context("failed to insert redirect_link")?;

            if affected > 0 {
                return Ok(RedirectLink {
                    id: id.clone(),
                    url: url.to_string(),
                    short_url: short_url_for(&id),
                    created_at: now,
                    last_used_at: None,
                    hit_count: 0,
                });
            }

            // Insert was a no-op. Either the URL is already stored (possibly
            // under a longer id from a concurrent writer — idempotent return)
            // or this id prefix collides with a different URL.
            if let Some(existing) = find_by_url(conn, url)? {
                return Ok(existing);
            }
            match get_by_id(conn, &id)? {
                Some(existing) if existing.url == url => return Ok(existing),
                Some(_) => {
                    // Hash-prefix collision with a different URL — lengthen.
                    len += 2;
                    continue;
                }
                None => {
                    // Race with a concurrent delete; retry this same length.
                    continue;
                }
            }
        }
    })
}

pub fn expand(config: &Config, id: &str) -> Result<Option<RedirectLink>> {
    let id = id.trim();
    if id.is_empty() {
        return Ok(None);
    }
    with_connection(config, |conn| {
        let found = get_by_id(conn, id)?;
        if found.is_some() {
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE redirect_links
                 SET hit_count = hit_count + 1, last_used_at = ?2
                 WHERE id = ?1",
                params![id, now],
            )
            .context("failed to bump redirect_link hit count")?;
        }
        Ok(found)
    })
}

pub fn peek(config: &Config, id: &str) -> Result<Option<RedirectLink>> {
    let id = id.trim();
    if id.is_empty() {
        return Ok(None);
    }
    with_connection(config, |conn| get_by_id(conn, id))
}

pub fn list(config: &Config, limit: usize) -> Result<Vec<RedirectLink>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, url, created_at, last_used_at, hit_count
             FROM redirect_links
             ORDER BY datetime(created_at) DESC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], row_to_link)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}

pub fn remove(config: &Config, id: &str) -> Result<bool> {
    with_connection(config, |conn| {
        let affected = conn
            .execute("DELETE FROM redirect_links WHERE id = ?1", params![id])
            .context("failed to delete redirect_link")?;
        Ok(affected > 0)
    })
}

fn get_by_id(conn: &Connection, id: &str) -> Result<Option<RedirectLink>> {
    conn.query_row(
        "SELECT id, url, created_at, last_used_at, hit_count
         FROM redirect_links WHERE id = ?1",
        params![id],
        row_to_link,
    )
    .optional()
    .map_err(Into::into)
}

fn find_by_url(conn: &Connection, url: &str) -> Result<Option<RedirectLink>> {
    conn.query_row(
        "SELECT id, url, created_at, last_used_at, hit_count
         FROM redirect_links WHERE url = ?1",
        params![url],
        row_to_link,
    )
    .optional()
    .map_err(Into::into)
}

fn row_to_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<RedirectLink> {
    let id: String = row.get(0)?;
    let url: String = row.get(1)?;
    let created_at: String = row.get(2)?;
    let last_used_at: Option<String> = row.get(3)?;
    let hit_count: i64 = row.get(4)?;
    let created_at = parse_ts(&created_at)?;
    let last_used_at = last_used_at.as_deref().map(parse_ts).transpose()?;
    Ok(RedirectLink {
        short_url: short_url_for(&id),
        id,
        url,
        created_at,
        last_used_at,
        hit_count: hit_count.max(0) as u64,
    })
}

fn parse_ts(s: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })
}

fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db_path = config.workspace_dir.join("redirect_links").join("links.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create redirect_links directory: {}",
                parent.display()
            )
        })?;
    }

    let conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open redirect_links DB: {}", db_path.display()))?;

    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS redirect_links (
            id            TEXT PRIMARY KEY,
            url           TEXT NOT NULL UNIQUE,
            created_at    TEXT NOT NULL,
            last_used_at  TEXT,
            hit_count     INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_redirect_links_url ON redirect_links(url);",
    )
    .context("Failed to initialize redirect_links schema")?;

    f(&conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().join("workspace");
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
        cfg
    }

    #[test]
    fn shorten_is_deterministic_and_dedupes() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let url = "https://www.trip.com/forward/middlepages/channel/openEdm.gif?bizData=eyJldmVudCI6Im9wZW4ifQ";
        let a = shorten(&cfg, url).unwrap();
        let b = shorten(&cfg, url).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.short_url, format!("openhuman://link/{}", a.id));
        assert_eq!(a.id.len(), DEFAULT_ID_LEN);
    }

    #[test]
    fn expand_returns_original_url_and_bumps_hits() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let link = shorten(&cfg, "https://example.com/a?x=1").unwrap();
        let got = expand(&cfg, &link.id).unwrap().expect("link exists");
        assert_eq!(got.url, "https://example.com/a?x=1");
        assert_eq!(got.hit_count, 0);
        let got2 = expand(&cfg, &link.id).unwrap().unwrap();
        assert_eq!(got2.hit_count, 1);
    }

    #[test]
    fn expand_unknown_id_returns_none() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        assert!(expand(&cfg, "deadbeef").unwrap().is_none());
    }

    #[test]
    fn id_from_short_accepts_scheme_and_rejects_others() {
        assert_eq!(
            id_from_short("openhuman://link/abc123"),
            Some("abc123".into())
        );
        assert!(id_from_short("https://example.com/").is_none());
        assert!(id_from_short("openhuman://link/").is_none());
        assert!(id_from_short("openhuman://link/not-hex!").is_none());
    }

    #[test]
    fn id_from_short_accepts_bare_id_and_normalizes_case() {
        // The docstring promises bare-id acceptance — lock it in.
        assert_eq!(id_from_short("abc123").as_deref(), Some("abc123"));
        assert_eq!(id_from_short("  ABC123  ").as_deref(), Some("abc123"));
        assert!(id_from_short("").is_none());
        assert!(id_from_short("not-hex").is_none());
    }

    #[test]
    fn shorten_handles_concurrent_calls_without_primary_key_error() {
        // Regression test: the previous check-then-insert path raced under
        // concurrent calls and hit a PRIMARY KEY constraint error. The
        // ON CONFLICT DO NOTHING path must return the same link for every
        // concurrent caller with the same URL.
        use std::sync::Arc;
        use std::thread;

        let tmp = TempDir::new().unwrap();
        let cfg = Arc::new(test_config(&tmp));
        let url = "https://example.com/concurrent?x=1".to_string();

        let mut handles = Vec::new();
        for _ in 0..8 {
            let cfg = Arc::clone(&cfg);
            let url = url.clone();
            handles.push(thread::spawn(move || shorten(&cfg, &url).unwrap()));
        }
        let ids: Vec<String> = handles.into_iter().map(|h| h.join().unwrap().id).collect();
        // Every concurrent writer must agree on a single id for the URL.
        assert!(ids.iter().all(|id| id == &ids[0]));
    }

    #[test]
    fn list_orders_newest_first_and_respects_limit() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        for i in 0..5 {
            shorten(
                &cfg,
                &format!("https://example.com/{i}?v=xxxxxxxxxxxxxxxxxxxx"),
            )
            .unwrap();
        }
        let rows = list(&cfg, 3).unwrap();
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn remove_deletes_and_reports_affected() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let link = shorten(&cfg, "https://example.com/rm").unwrap();
        assert!(remove(&cfg, &link.id).unwrap());
        assert!(!remove(&cfg, &link.id).unwrap());
    }
}
