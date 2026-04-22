# Life-Capture Plan #1 — Foundation Milestone

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the local personal-index foundation that every later life-capture feature reads and writes — a SQLite + sqlite-vec store, a pluggable Embedder trait with a hosted OpenAI implementation, hybrid (vector + keyword + recency) retrieval, PII redaction, quoted-thread stripping, and a controller-registered RPC surface — all with no ingestion or UI yet, exercised end-to-end via an integration test using synthetic items.

**Architecture:** A new domain module at `src/openhuman/life_capture/`. Items land in a local SQLite database `personal_index.db` in the user's app data directory. Vectors live in a `vec0` virtual table from the sqlite-vec extension; full-text lives in an FTS5 table; both are kept in sync with the canonical `items` row via triggers. The `Embedder` trait isolates the network call so swapping providers (and adding a local model in milestone 4) is one impl, not a rewrite. Public surface is exposed only through controller schemas under `life_capture/schemas.rs` per the repo's controller-only-exposure rule.

**Tech Stack:** Rust 1.93, sqlx 0.7+ (SQLite, runtime-tokio-rustls), sqlite-vec extension (loaded at runtime), rusqlite-style triggers via raw SQL, regex 1.x, reqwest (already in tree), tokio.

---

## File structure

| File | Responsibility |
|---|---|
| `src/openhuman/life_capture/mod.rs` | Light re-export hub per repo rules; no logic |
| `src/openhuman/life_capture/types.rs` | `Item`, `Source`, `Person`, `Hit`, `Query`, `IndexStats` |
| `src/openhuman/life_capture/index.rs` | SQLite + sqlite-vec store; `IndexWriter` + `IndexReader` impls |
| `src/openhuman/life_capture/migrations.rs` | Embedded SQL migrations (versioned) — v1 ships items + items_fts + sync_state + ACL column (Onyx pattern) |
| `src/openhuman/life_capture/embedder.rs` | `Embedder` trait + `HostedEmbedder` impl (OpenAI) |
| `src/openhuman/life_capture/redact.rs` | PII redaction (regex, no model dependency) |
| `src/openhuman/life_capture/quote_strip.rs` | Email quoted-reply stripping |
| `src/openhuman/life_capture/schemas.rs` | Controller schemas — `LifeCaptureController` exposing `get_status`, `get_stats`, `search`, `embed_and_store` (test-only) |
| `src/openhuman/life_capture/rpc.rs` | `RpcOutcome`-style controller handlers calling into the module |
| `src/openhuman/life_capture/tests/e2e.rs` | End-to-end test wiring all the pieces with a mocked embedder |
| `src/openhuman/life_capture/tests/fixtures/` | Static fixture inputs (sample emails, expected redactions) |

---

## Task F1: Module skeleton, dependencies, and registration

**Files:**
- Create: `src/openhuman/life_capture/mod.rs`
- Create: `src/openhuman/life_capture/types.rs` (empty stub)
- Modify: `src/openhuman/mod.rs` (add `pub mod life_capture;`)
- Modify: `Cargo.toml` (add deps)

- [ ] **Step 1: Add dependencies in `Cargo.toml`**

```toml
[dependencies]
# (additions only — keep existing deps untouched)
sqlx = { version = "0.7", features = ["runtime-tokio-rustls", "sqlite", "macros", "migrate", "chrono", "uuid"] }
sqlite-vec = "0.1"
regex = "1"
once_cell = "1"
```

- [ ] **Step 2: Create the module skeleton**

`src/openhuman/life_capture/mod.rs`:

```rust
pub mod embedder;
pub mod index;
pub mod migrations;
pub mod quote_strip;
pub mod redact;
pub mod rpc;
pub mod schemas;
pub mod types;

pub use embedder::{Embedder, HostedEmbedder};
pub use index::{IndexReader, IndexWriter, PersonalIndex};
pub use types::{Hit, IndexStats, Item, Person, Query, Source};
```

`src/openhuman/life_capture/types.rs`:

```rust
// Filled in by Task F2.
```

`src/openhuman/life_capture/{embedder,index,migrations,quote_strip,redact,rpc,schemas}.rs`:

```rust
// Filled in by later tasks.
```

- [ ] **Step 3: Register the module in `src/openhuman/mod.rs`**

Add a single line in alphabetical order with the existing `pub mod` lines:

```rust
pub mod life_capture;
```

- [ ] **Step 4: Verify the workspace still compiles**

```bash
cargo check --manifest-path Cargo.toml
```
Expected: clean compile (just empty modules; warnings about unused stubs are fine).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/openhuman/mod.rs src/openhuman/life_capture/
git commit -m "feat(life_capture): module skeleton + sqlx/sqlite-vec/regex deps"
```

---

## Task F2: Core types

**Files:**
- Modify: `src/openhuman/life_capture/types.rs`

- [ ] **Step 1: Write a failing test**

`src/openhuman/life_capture/types.rs` (test at the bottom):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_round_trips_via_serde_json() {
        let item = Item {
            id: uuid::Uuid::nil(),
            source: Source::Gmail,
            external_id: "gmail-thread-123/msg-1".into(),
            ts: chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            author: Some(Person {
                display_name: Some("Sarah Lee".into()),
                email: Some("sarah@example.com".into()),
                source_id: Some("UABCD".into()),
            }),
            subject: Some("Ledger contract draft".into()),
            text: "Hi — attached is the draft.".into(),
            metadata: serde_json::json!({"thread_id": "abc"}),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: Item = serde_json::from_str(&json).unwrap();
        assert_eq!(back.external_id, item.external_id);
        assert_eq!(back.source, Source::Gmail);
        assert_eq!(back.author.unwrap().email.as_deref(), Some("sarah@example.com"));
    }

    #[test]
    fn source_serializes_as_lowercase_string() {
        assert_eq!(serde_json::to_string(&Source::IMessage).unwrap(), "\"imessage\"");
        let back: Source = serde_json::from_str("\"calendar\"").unwrap();
        assert_eq!(back, Source::Calendar);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::types -- --nocapture
```
Expected: FAIL — `Item`, `Source`, `Person` are not defined.

- [ ] **Step 3: Implement the types**

Replace the body of `src/openhuman/life_capture/types.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single ingested item. One canonical shape for every source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: Uuid,
    pub source: Source,
    /// Source-specific dedupe key. (gmail msg-id, calendar event id, imessage rowid, ...)
    pub external_id: String,
    pub ts: DateTime<Utc>,
    pub author: Option<Person>,
    pub subject: Option<String>,
    /// Normalized, redacted, quote-stripped body.
    pub text: String,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Gmail,
    Calendar,
    IMessage,
    Slack, // present in the type but not ingested in v1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    pub display_name: Option<String>,
    pub email: Option<String>,
    /// Source-native id (gmail address, calendar attendee email, imessage handle).
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    pub text: String,
    pub k: usize,
    pub sources: Vec<Source>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
}

impl Query {
    pub fn simple(text: impl Into<String>, k: usize) -> Self {
        Self { text: text.into(), k, sources: vec![], since: None, until: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub item: Item,
    pub score: f32,
    /// Short surrounding text for citation rendering.
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub total_items: u64,
    pub by_source: Vec<(Source, u64)>,
    pub last_ingest_ts: Option<DateTime<Utc>>,
}
```

- [ ] **Step 4: Run test, then commit**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::types -- --nocapture
```
Expected: PASS.

```bash
git add src/openhuman/life_capture/types.rs
git commit -m "feat(life_capture): core types — Item, Source, Person, Query, Hit, IndexStats"
```

---

## Task F3: SQLite schema and migrations

**Files:**
- Modify: `src/openhuman/life_capture/migrations.rs`
- Create: `src/openhuman/life_capture/migrations/0001_init.sql`

- [ ] **Step 1: Write the migration SQL**

`src/openhuman/life_capture/migrations/0001_init.sql`:

```sql
CREATE TABLE IF NOT EXISTS items (
    id                  TEXT PRIMARY KEY,           -- uuid
    source              TEXT NOT NULL,
    external_id         TEXT NOT NULL,
    ts                  INTEGER NOT NULL,           -- unix seconds
    author_json         TEXT,                       -- serialized Person, nullable
    subject             TEXT,
    text                TEXT NOT NULL,              -- normalized body for embedding (semantic)
    text_keyword        TEXT,                       -- raw values projection for FTS5/BM25 (Onyx pattern)
    metadata_json       TEXT NOT NULL DEFAULT '{}',
    -- ACL tokens, e.g. ["user:local","source:gmail","account:foo@bar"]. Single-user v1 still
    -- sets ["user:local"] so the column is mandatory and team v2 needs no migration.
    access_control_list TEXT NOT NULL DEFAULT '["user:local"]',
    UNIQUE(source, external_id)
);

-- Per-connector checkpoint store. Plan #2 ingestors write here; Foundation creates the table
-- so the schema is in place when ingestors land.
CREATE TABLE IF NOT EXISTS sync_state (
    connector_name TEXT PRIMARY KEY,    -- e.g. "gmail", "calendar", "imessage"
    cursor_blob    TEXT,                -- opaque per-connector cursor (JSON)
    last_sync_ts   INTEGER,
    last_error     TEXT
);

CREATE INDEX IF NOT EXISTS items_source_ts_idx ON items(source, ts DESC);
CREATE INDEX IF NOT EXISTS items_ts_idx        ON items(ts DESC);

-- Full-text companion table.
CREATE VIRTUAL TABLE IF NOT EXISTS items_fts USING fts5(
    subject,
    text,
    content='items',
    content_rowid='rowid',
    tokenize='porter unicode61'
);

-- Keep FTS in sync via triggers.
CREATE TRIGGER IF NOT EXISTS items_ai AFTER INSERT ON items BEGIN
    INSERT INTO items_fts(rowid, subject, text)
    VALUES (new.rowid, COALESCE(new.subject, ''), new.text);
END;

CREATE TRIGGER IF NOT EXISTS items_ad AFTER DELETE ON items BEGIN
    INSERT INTO items_fts(items_fts, rowid, subject, text)
    VALUES ('delete', old.rowid, COALESCE(old.subject, ''), old.text);
END;

CREATE TRIGGER IF NOT EXISTS items_au AFTER UPDATE ON items BEGIN
    INSERT INTO items_fts(items_fts, rowid, subject, text)
    VALUES ('delete', old.rowid, COALESCE(old.subject, ''), old.text);
    INSERT INTO items_fts(rowid, subject, text)
    VALUES (new.rowid, COALESCE(new.subject, ''), new.text);
END;
```

- [ ] **Step 2: Implement the migrations runner**

`src/openhuman/life_capture/migrations.rs`:

```rust
use sqlx::SqlitePool;

const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("migrations/0001_init.sql")),
];

pub async fn run(pool: &SqlitePool) -> sqlx::Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _life_capture_migrations (
            name TEXT PRIMARY KEY,
            applied_at INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;

    for (name, sql) in MIGRATIONS {
        let already: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM _life_capture_migrations WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(pool)
        .await?;
        if already.is_some() {
            continue;
        }
        let mut tx = pool.begin().await?;
        sqlx::raw_sql(sql).execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO _life_capture_migrations(name, applied_at) VALUES (?, strftime('%s','now'))",
        )
        .bind(name)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }
    Ok(())
}
```

- [ ] **Step 3: Write a test that runs migrations against an in-memory DB**

Append to `src/openhuman/life_capture/migrations.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn migrations_apply_idempotently() {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("connect");
        run(&pool).await.expect("first run");
        run(&pool).await.expect("second run idempotent");

        let count: (i64,) =
            sqlx::query_as("SELECT count(*) FROM _life_capture_migrations")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count.0, MIGRATIONS.len() as i64);

        // FTS table exists and accepts inserts via the trigger.
        sqlx::query(
            "INSERT INTO items(id, source, external_id, ts, text)
             VALUES ('00000000-0000-0000-0000-000000000001', 'gmail', 'msg-1', 0, 'hello world')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let hit: (i64,) =
            sqlx::query_as("SELECT count(*) FROM items_fts WHERE items_fts MATCH 'hello'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(hit.0, 1);
    }
}
```

- [ ] **Step 4: Run the test and commit**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::migrations -- --nocapture
```
Expected: PASS.

```bash
git add src/openhuman/life_capture/migrations.rs src/openhuman/life_capture/migrations/0001_init.sql
git commit -m "feat(life_capture): items + items_fts schema and migrations runner"
```

---

## Task F4: sqlite-vec extension loading and vector schema

**Files:**
- Create: `src/openhuman/life_capture/migrations/0002_vec.sql`
- Modify: `src/openhuman/life_capture/migrations.rs` (add to MIGRATIONS list)
- Create: a connector helper in `src/openhuman/life_capture/index.rs` (loads extension)

- [ ] **Step 1: Add the vec0 migration**

`src/openhuman/life_capture/migrations/0002_vec.sql`:

```sql
-- 1536-dim vectors for OpenAI text-embedding-3-small.
CREATE VIRTUAL TABLE IF NOT EXISTS item_vectors USING vec0(
    item_id TEXT PRIMARY KEY,
    embedding float[1536]
);
```

- [ ] **Step 2: Add the migration to the list**

In `src/openhuman/life_capture/migrations.rs`:

```rust
const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("migrations/0001_init.sql")),
    ("0002_vec",  include_str!("migrations/0002_vec.sql")),
];
```

- [ ] **Step 3: Add a connector helper that loads the sqlite-vec extension**

`src/openhuman/life_capture/index.rs`:

```rust
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::str::FromStr;

pub struct PersonalIndex {
    pub pool: SqlitePool,
}

impl PersonalIndex {
    /// Open (or create) the personal index at `path`. Loads sqlite-vec, runs migrations.
    pub async fn open(path: &Path) -> anyhow::Result<Self> {
        let url = format!("sqlite://{}?mode=rwc", path.display());
        let opts = SqliteConnectOptions::from_str(&url)?
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .foreign_keys(true);

        // sqlx 0.7 supports loading extensions via after_connect.
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .after_connect(|conn, _meta| Box::pin(async move {
                // Load sqlite-vec from the linked C library.
                unsafe {
                    let raw = conn.handle().lock_handle().await?;
                    let rc = sqlite_vec::sqlite3_vec_init(raw.as_raw_handle(), std::ptr::null_mut(), std::ptr::null());
                    if rc != 0 {
                        return Err(sqlx::Error::Configuration("sqlite-vec init failed".into()));
                    }
                }
                Ok(())
            }))
            .connect_with(opts)
            .await?;

        super::migrations::run(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn open_in_memory() -> anyhow::Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .after_connect(|conn, _meta| Box::pin(async move {
                unsafe {
                    let raw = conn.handle().lock_handle().await?;
                    let rc = sqlite_vec::sqlite3_vec_init(raw.as_raw_handle(), std::ptr::null_mut(), std::ptr::null());
                    if rc != 0 {
                        return Err(sqlx::Error::Configuration("sqlite-vec init failed".into()));
                    }
                }
                Ok(())
            }))
            .connect("sqlite::memory:")
            .await?;
        super::migrations::run(&pool).await?;
        Ok(Self { pool })
    }
}
```

(If the sqlite-vec crate version pinned in `Cargo.toml` exposes a different init helper, use what it provides. The shape is: load the extension before the first migration runs.)

- [ ] **Step 4: Test that vec0 is callable**

In `src/openhuman/life_capture/index.rs`, add:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn vec0_table_accepts_insert_and_returns_distance() {
        let idx = PersonalIndex::open_in_memory().await.expect("open");
        sqlx::query(
            "INSERT INTO item_vectors(item_id, embedding) VALUES (?, ?)",
        )
        .bind("00000000-0000-0000-0000-000000000001")
        .bind(serde_json::to_string(&vec![0.0_f32; 1536]).unwrap())
        .execute(&idx.pool)
        .await
        .expect("insert vec");

        let row: (String,) = sqlx::query_as(
            "SELECT item_id FROM item_vectors WHERE embedding MATCH ? ORDER BY distance LIMIT 1",
        )
        .bind(serde_json::to_string(&vec![0.0_f32; 1536]).unwrap())
        .fetch_one(&idx.pool)
        .await
        .expect("query vec");
        assert_eq!(row.0, "00000000-0000-0000-0000-000000000001");
    }
}
```

- [ ] **Step 5: Run the test and commit**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::index -- --nocapture
```
Expected: PASS.

```bash
git add src/openhuman/life_capture/migrations/0002_vec.sql src/openhuman/life_capture/migrations.rs src/openhuman/life_capture/index.rs
git commit -m "feat(life_capture): sqlite-vec extension loading + 1536d item_vectors table"
```

---

## Task F5: IndexWriter — upsert with dedupe

**Files:**
- Modify: `src/openhuman/life_capture/index.rs`

- [ ] **Step 1: Write a failing test**

Append to `src/openhuman/life_capture/index.rs`:

```rust
#[cfg(test)]
mod writer_tests {
    use super::*;
    use crate::openhuman::life_capture::types::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn sample_item(ext: &str, text: &str) -> Item {
        Item {
            id: Uuid::new_v4(),
            source: Source::Gmail,
            external_id: ext.into(),
            ts: Utc::now(),
            author: None,
            subject: Some("subj".into()),
            text: text.into(),
            metadata: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn upsert_inserts_new_and_dedupes_existing() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let writer = IndexWriter::new(&idx);

        writer.upsert(&[sample_item("a", "first")]).await.unwrap();
        writer.upsert(&[sample_item("a", "first updated")]).await.unwrap();
        writer.upsert(&[sample_item("b", "second")]).await.unwrap();

        let count: (i64,) = sqlx::query_as("SELECT count(*) FROM items")
            .fetch_one(&idx.pool).await.unwrap();
        assert_eq!(count.0, 2, "dedupe by (source, external_id)");

        let updated: (String,) =
            sqlx::query_as("SELECT text FROM items WHERE external_id='a'")
                .fetch_one(&idx.pool).await.unwrap();
        assert_eq!(updated.0, "first updated", "upsert overwrites text");
    }
}
```

- [ ] **Step 2: Run, verify failure**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::index::writer_tests -- --nocapture
```
Expected: FAIL — `IndexWriter` is not defined.

- [ ] **Step 3: Implement `IndexWriter`**

Append to `src/openhuman/life_capture/index.rs`:

```rust
use crate::openhuman::life_capture::types::Item;

pub struct IndexWriter<'a> {
    pool: &'a SqlitePool,
}

impl<'a> IndexWriter<'a> {
    pub fn new(idx: &'a PersonalIndex) -> Self {
        Self { pool: &idx.pool }
    }

    /// Upserts items by (source, external_id). Vectors are written separately
    /// (see Task F8) — this only writes the canonical row + FTS via triggers.
    pub async fn upsert(&self, items: &[Item]) -> sqlx::Result<()> {
        let mut tx = self.pool.begin().await?;
        for item in items {
            let author_json = item.author.as_ref().map(|a| serde_json::to_string(a).unwrap());
            let metadata_json = serde_json::to_string(&item.metadata).unwrap();
            let source = serde_json::to_value(&item.source).unwrap().as_str().unwrap().to_string();

            sqlx::query(
                "INSERT INTO items(id, source, external_id, ts, author_json, subject, text, metadata_json)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(source, external_id) DO UPDATE SET
                   ts            = excluded.ts,
                   author_json   = excluded.author_json,
                   subject       = excluded.subject,
                   text          = excluded.text,
                   metadata_json = excluded.metadata_json"
            )
            .bind(item.id.to_string())
            .bind(&source)
            .bind(&item.external_id)
            .bind(item.ts.timestamp())
            .bind(author_json)
            .bind(item.subject.as_deref())
            .bind(&item.text)
            .bind(&metadata_json)
            .execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Writes a vector for an existing item. Replaces if present.
    pub async fn upsert_vector(&self, item_id: &uuid::Uuid, vector: &[f32]) -> sqlx::Result<()> {
        let id_s = item_id.to_string();
        let v_json = serde_json::to_string(vector).unwrap();
        sqlx::query("DELETE FROM item_vectors WHERE item_id = ?")
            .bind(&id_s).execute(self.pool).await?;
        sqlx::query("INSERT INTO item_vectors(item_id, embedding) VALUES (?, ?)")
            .bind(&id_s).bind(&v_json).execute(self.pool).await?;
        Ok(())
    }
}
```

- [ ] **Step 4: Verify test passes**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::index::writer_tests -- --nocapture
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/openhuman/life_capture/index.rs
git commit -m "feat(life_capture): IndexWriter — upsert items with (source, external_id) dedupe"
```

---

## Task F6: PII redaction utility

**Files:**
- Modify: `src/openhuman/life_capture/redact.rs`

- [ ] **Step 1: Write the failing test**

`src/openhuman/life_capture/redact.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_emails_phones_ssn_and_credit_cards() {
        let cases = [
            ("contact me at sarah@example.com today",
             "contact me at <EMAIL> today"),
            ("call (415) 555-0123 or +1-415-555-0123",
             "call <PHONE> or <PHONE>"),
            ("ssn 123-45-6789 then",
             "ssn <SSN> then"),
            ("card 4111-1111-1111-1111 expires",
             "card <CC> expires"),
            ("nothing sensitive here", "nothing sensitive here"),
        ];
        for (input, expected) in cases {
            assert_eq!(redact(input), expected, "input: {input}");
        }
    }

    #[test]
    fn idempotent_on_already_redacted_text() {
        let s = "see <EMAIL> and <PHONE>";
        assert_eq!(redact(s), s);
    }
}
```

- [ ] **Step 2: Verify failure**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::redact -- --nocapture
```
Expected: FAIL — `redact` not defined.

- [ ] **Step 3: Implement**

Replace `src/openhuman/life_capture/redact.rs`:

```rust
use once_cell::sync::Lazy;
use regex::Regex;

static EMAIL: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").unwrap()
});

// Catches +1-415-555-0123, (415) 555-0123, 415.555.0123, 4155550123 in 10-15 digit forms.
static PHONE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?x)
        \+?\d{1,3}[\s\-.]?  # optional country code
        (?:\(\d{2,4}\)|\d{2,4})[\s\-.]?
        \d{3}[\s\-.]?\d{3,4}
    ").unwrap()
});

static SSN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap()
});

// Matches typical 13-19 digit credit card numbers with dashes/spaces every 4.
static CC: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b(?:\d[ \-]?){12,18}\d\b").unwrap()
});

/// Apply best-effort PII redaction. Order matters: emails first (so phone regex
/// doesn't eat the local-part of an email's digit run), then SSN (specific shape),
/// then CC (long digit runs), then phone.
pub fn redact(input: &str) -> String {
    let s = EMAIL.replace_all(input, "<EMAIL>").into_owned();
    let s = SSN.replace_all(&s, "<SSN>").into_owned();
    let s = CC.replace_all(&s, "<CC>").into_owned();
    let s = PHONE.replace_all(&s, "<PHONE>").into_owned();
    s
}
```

- [ ] **Step 4: Run test, iterate until green**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::redact -- --nocapture
```
Expected: PASS. If a regex over-matches in the test (e.g. `<EMAIL>` getting partially eaten), tighten the pattern. Do not weaken the test.

- [ ] **Step 5: Commit**

```bash
git add src/openhuman/life_capture/redact.rs
git commit -m "feat(life_capture): regex-based PII redaction (email/phone/ssn/cc)"
```

---

## Task F7: Quoted-thread stripping

**Files:**
- Modify: `src/openhuman/life_capture/quote_strip.rs`

- [ ] **Step 1: Write the failing test**

`src/openhuman/life_capture/quote_strip.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_on_date_wrote_block_and_below() {
        let input = "Quick reply.\n\nOn Mon, Apr 21, 2026 at 9:14 AM, Sarah <sarah@x> wrote:\n> earlier text\n>> deeper\n";
        assert_eq!(strip_quoted_reply(input), "Quick reply.");
    }

    #[test]
    fn strips_lines_starting_with_gt() {
        let input = "new thought\n> old thought\n> > older\nnewer\n";
        assert_eq!(strip_quoted_reply(input), "new thought\nnewer");
    }

    #[test]
    fn strips_outlook_original_message_separator() {
        let input = "reply text\n\n-----Original Message-----\nFrom: a@b\nTo: c@d\nSubject: ...\nbody";
        assert_eq!(strip_quoted_reply(input), "reply text");
    }

    #[test]
    fn passthrough_when_no_quote_found() {
        let s = "single paragraph no markers";
        assert_eq!(strip_quoted_reply(s), s);
    }
}
```

- [ ] **Step 2: Verify failure**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::quote_strip -- --nocapture
```
Expected: FAIL — function undefined.

- [ ] **Step 3: Implement**

Replace `src/openhuman/life_capture/quote_strip.rs`:

```rust
use once_cell::sync::Lazy;
use regex::Regex;

static ON_DATE_WROTE: Lazy<Regex> = Lazy::new(|| {
    // "On <date>, <name> <email|>(?) wrote:" — match start of any line.
    Regex::new(r"(?m)^On .{1,200}\bwrote:\s*$").unwrap()
});

static OUTLOOK_SEP: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^-{3,}\s*Original Message\s*-{3,}\s*$").unwrap()
});

/// Returns only the new content of an email body — drops everything from the
/// first quoted-reply marker onward, plus any line that begins with '>'.
pub fn strip_quoted_reply(body: &str) -> String {
    // Cut at the earliest of the two marker types.
    let mut cut: Option<usize> = None;
    for re in [&*ON_DATE_WROTE, &*OUTLOOK_SEP] {
        if let Some(m) = re.find(body) {
            cut = Some(cut.map_or(m.start(), |c| c.min(m.start())));
        }
    }
    let head = if let Some(idx) = cut { &body[..idx] } else { body };

    // Drop any line starting with '>'.
    let kept: Vec<&str> = head.lines().filter(|l| !l.trim_start().starts_with('>')).collect();
    kept.join("\n").trim().to_string()
}
```

- [ ] **Step 4: Run, iterate until tests pass**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::quote_strip -- --nocapture
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/openhuman/life_capture/quote_strip.rs
git commit -m "feat(life_capture): strip quoted reply chains (on-date-wrote, outlook, gt-prefix)"
```

---

## Task F8: Embedder trait + HostedEmbedder (OpenAI)

**Files:**
- Modify: `src/openhuman/life_capture/embedder.rs`

- [ ] **Step 1: Write the failing test using a fake HTTP server**

`src/openhuman/life_capture/embedder.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    #[tokio::test]
    async fn hosted_embedder_calls_openai_compatible_endpoint() {
        let server = MockServer::start_async().await;
        let body = serde_json::json!({
            "data": [
                {"index": 0, "embedding": vec![0.1_f32; 1536]},
                {"index": 1, "embedding": vec![0.2_f32; 1536]},
            ],
            "model": "text-embedding-3-small",
            "usage": {"prompt_tokens": 4, "total_tokens": 4}
        });
        let mock = server.mock_async(|when, then| {
            when.method(POST).path("/v1/embeddings");
            then.status(200).header("content-type", "application/json").json_body(body);
        }).await;

        let emb = HostedEmbedder::new(
            format!("{}/v1", server.base_url()),
            "test-key".into(),
            "text-embedding-3-small".into(),
        );
        let out = emb.embed_batch(&["hello", "world"]).await.expect("embed");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].len(), 1536);
        assert!((out[0][0] - 0.1).abs() < 1e-6);
        mock.assert_async().await;
    }
}
```

Add to `[dev-dependencies]` in `Cargo.toml`:

```toml
httpmock = "0.7"
```

- [ ] **Step 2: Verify failure**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::embedder -- --nocapture
```
Expected: FAIL — `Embedder` / `HostedEmbedder` undefined.

- [ ] **Step 3: Implement**

Replace `src/openhuman/life_capture/embedder.rs`:

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[async_trait]
pub trait Embedder: Send + Sync {
    /// Returns vectors in the same order as `texts`. All vectors share `dim()`.
    async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>>;
    fn dim(&self) -> usize;
}

#[derive(Clone)]
pub struct HostedEmbedder {
    base_url: String,
    api_key: String,
    model: String,
    http: reqwest::Client,
}

impl HostedEmbedder {
    pub fn new(base_url: String, api_key: String, model: String) -> Self {
        Self { base_url, api_key, model, http: reqwest::Client::new() }
    }
}

#[derive(Serialize)]
struct EmbedReq<'a> { input: &'a [&'a str], model: &'a str }

#[derive(Deserialize)]
struct EmbedRespItem { #[allow(dead_code)] index: usize, embedding: Vec<f32> }

#[derive(Deserialize)]
struct EmbedResp { data: Vec<EmbedRespItem> }

#[async_trait]
impl Embedder for HostedEmbedder {
    async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let url = format!("{}/embeddings", self.base_url.trim_end_matches('/'));
        let resp = self.http
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&EmbedReq { input: texts, model: &self.model })
            .send()
            .await?
            .error_for_status()?
            .json::<EmbedResp>()
            .await?;
        let mut data = resp.data;
        // Keep order by index just in case the server returns out of order.
        data.sort_by_key(|d| d.index);
        Ok(data.into_iter().map(|d| d.embedding).collect())
    }

    fn dim(&self) -> usize { 1536 }
}
```

Add to `[dependencies]` in `Cargo.toml` (if not already present):

```toml
async-trait = "0.1"
```

- [ ] **Step 4: Run, verify pass**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::embedder -- --nocapture
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock src/openhuman/life_capture/embedder.rs
git commit -m "feat(life_capture): Embedder trait + HostedEmbedder (OpenAI-compatible)"
```

---

## Task F9: IndexReader — keyword search via FTS5

**Files:**
- Modify: `src/openhuman/life_capture/index.rs`

- [ ] **Step 1: Write the failing test**

Append to `src/openhuman/life_capture/index.rs`:

```rust
#[cfg(test)]
mod reader_keyword_tests {
    use super::*;
    use crate::openhuman::life_capture::types::*;
    use chrono::{TimeZone, Utc};
    use uuid::Uuid;

    fn it(ext: &str, subj: &str, text: &str, ts_secs: i64) -> Item {
        Item {
            id: Uuid::new_v4(), source: Source::Gmail,
            external_id: ext.into(),
            ts: Utc.timestamp_opt(ts_secs, 0).single().unwrap(),
            author: None, subject: Some(subj.into()), text: text.into(),
            metadata: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn keyword_search_ranks_by_relevance() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let writer = IndexWriter::new(&idx);
        writer.upsert(&[
            it("a", "ledger contract", "the ledger contract draft is attached", 100),
            it("b", "lunch",           "let's grab lunch", 200),
            it("c", "ledger",          "ledger ledger ledger", 300),
        ]).await.unwrap();

        let reader = IndexReader::new(&idx);
        let hits = reader.keyword_search("ledger contract", 10).await.unwrap();
        assert!(!hits.is_empty(), "expected at least one hit");
        assert_eq!(hits[0].item.external_id, "a", "best match should be a");
    }
}
```

- [ ] **Step 2: Verify failure**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::index::reader_keyword_tests -- --nocapture
```
Expected: FAIL — `IndexReader` undefined.

- [ ] **Step 3: Implement keyword search**

Append to `src/openhuman/life_capture/index.rs`:

```rust
use crate::openhuman::life_capture::types::{Hit, Item};

pub struct IndexReader<'a> {
    pool: &'a SqlitePool,
}

impl<'a> IndexReader<'a> {
    pub fn new(idx: &'a PersonalIndex) -> Self { Self { pool: &idx.pool } }

    pub async fn keyword_search(&self, query: &str, k: usize) -> sqlx::Result<Vec<Hit>> {
        // bm25(items_fts) is negative in sqlite (lower is better); we negate so higher = better.
        // ACL filter: v1 always passes ["user:local"]; expressed as a json_each EXISTS clause so
        // the same query shape works for team v2 with multi-token ACLs.
        let rows = sqlx::query_as::<_, ItemRow>(
            "SELECT i.id, i.source, i.external_id, i.ts, i.author_json, i.subject, i.text, i.metadata_json,
                    -bm25(items_fts) AS score,
                    snippet(items_fts, 1, '«', '»', '…', 12) AS snip
             FROM items_fts JOIN items i ON i.rowid = items_fts.rowid
             WHERE items_fts MATCH ?
               AND EXISTS (SELECT 1 FROM json_each(i.access_control_list) WHERE value = 'user:local')
             ORDER BY score DESC
             LIMIT ?"
        )
        .bind(query)
        .bind(k as i64)
        .fetch_all(self.pool).await?;
        Ok(rows.into_iter().map(ItemRow::into_hit).collect())
    }
}

#[derive(sqlx::FromRow)]
struct ItemRow {
    id: String,
    source: String,
    external_id: String,
    ts: i64,
    author_json: Option<String>,
    subject: Option<String>,
    text: String,
    metadata_json: String,
    score: f64,
    snip: String,
}

impl ItemRow {
    fn into_hit(self) -> Hit {
        let author = self.author_json
            .and_then(|s| serde_json::from_str(&s).ok());
        let metadata = serde_json::from_str(&self.metadata_json)
            .unwrap_or(serde_json::json!({}));
        let source: crate::openhuman::life_capture::types::Source =
            serde_json::from_value(serde_json::Value::String(self.source.clone()))
                .unwrap_or(crate::openhuman::life_capture::types::Source::Gmail);
        Hit {
            score: self.score as f32,
            snippet: self.snip,
            item: Item {
                id: uuid::Uuid::parse_str(&self.id).unwrap(),
                source,
                external_id: self.external_id,
                ts: chrono::DateTime::from_timestamp(self.ts, 0).unwrap(),
                author,
                subject: self.subject,
                text: self.text,
                metadata,
            },
        }
    }
}
```

- [ ] **Step 4: Run, verify pass**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::index::reader_keyword_tests -- --nocapture
```
Expected: PASS. If FTS5 ranking is unexpected, double-check the `MATCH` query (use `"ledger" AND "contract"` if bare-token query underperforms — but tests should reflect intent, not internals).

- [ ] **Step 5: Commit**

```bash
git add src/openhuman/life_capture/index.rs
git commit -m "feat(life_capture): IndexReader keyword search via FTS5 + bm25"
```

---

## Task F10: IndexReader — vector search

**Files:**
- Modify: `src/openhuman/life_capture/index.rs`

- [ ] **Step 1: Write the failing test**

Append to `src/openhuman/life_capture/index.rs`:

```rust
#[cfg(test)]
mod reader_vector_tests {
    use super::*;
    use crate::openhuman::life_capture::types::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn near(target: &[f32], jitter: f32) -> Vec<f32> {
        target.iter().map(|x| x + jitter).collect()
    }

    #[tokio::test]
    async fn vector_search_returns_nearest_first() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let writer = IndexWriter::new(&idx);

        let mut item = |ext: &str| Item {
            id: Uuid::new_v4(), source: Source::Gmail, external_id: ext.into(),
            ts: Utc::now(), author: None, subject: None,
            text: format!("body of {ext}"), metadata: serde_json::json!({}),
        };

        let a = item("a"); let b = item("b"); let c = item("c");
        writer.upsert(&[a.clone(), b.clone(), c.clone()]).await.unwrap();

        // Construct three orthogonal-ish vectors at 1536 dims.
        let mut va = vec![0.0_f32; 1536]; va[0] = 1.0;
        let mut vb = vec![0.0_f32; 1536]; vb[1] = 1.0;
        let mut vc = vec![0.0_f32; 1536]; vc[2] = 1.0;

        writer.upsert_vector(&a.id, &va).await.unwrap();
        writer.upsert_vector(&b.id, &vb).await.unwrap();
        writer.upsert_vector(&c.id, &vc).await.unwrap();

        let reader = IndexReader::new(&idx);
        let hits = reader.vector_search(&near(&va, 0.01), 2).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].item.external_id, "a", "nearest first");
    }
}
```

- [ ] **Step 2: Verify failure**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::index::reader_vector_tests -- --nocapture
```
Expected: FAIL — `vector_search` undefined.

- [ ] **Step 3: Implement vector search**

Add to `impl<'a> IndexReader<'a>` in `src/openhuman/life_capture/index.rs`:

```rust
    pub async fn vector_search(&self, vector: &[f32], k: usize) -> sqlx::Result<Vec<Hit>> {
        let v_json = serde_json::to_string(vector).unwrap();
        let rows = sqlx::query_as::<_, ItemRow>(
            "SELECT i.id, i.source, i.external_id, i.ts, i.author_json, i.subject, i.text, i.metadata_json,
                    (1.0 / (1.0 + v.distance)) AS score,
                    substr(i.text, 1, 200) AS snip
             FROM item_vectors v JOIN items i ON i.id = v.item_id
             WHERE v.embedding MATCH ?
             ORDER BY v.distance ASC
             LIMIT ?"
        )
        .bind(&v_json)
        .bind(k as i64)
        .fetch_all(self.pool).await?;
        Ok(rows.into_iter().map(ItemRow::into_hit).collect())
    }
```

- [ ] **Step 4: Run, verify pass, commit**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::index::reader_vector_tests -- --nocapture
```
Expected: PASS.

```bash
git add src/openhuman/life_capture/index.rs
git commit -m "feat(life_capture): IndexReader vector_search via sqlite-vec MATCH"
```

---

## Task F11: IndexReader — hybrid (vector + keyword + recency)

**Files:**
- Modify: `src/openhuman/life_capture/index.rs`

- [ ] **Step 1: Write the failing test**

Append to `src/openhuman/life_capture/index.rs`:

```rust
#[cfg(test)]
mod reader_hybrid_tests {
    use super::*;
    use crate::openhuman::life_capture::types::*;
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    #[tokio::test]
    async fn hybrid_combines_signals_and_breaks_keyword_only_ties_with_vector() {
        let idx = PersonalIndex::open_in_memory().await.unwrap();
        let w = IndexWriter::new(&idx);

        let now = Utc::now();
        let mk = |ext: &str, subj: &str, text: &str, days_ago: i64| Item {
            id: Uuid::new_v4(), source: Source::Gmail, external_id: ext.into(),
            ts: now - Duration::days(days_ago), author: None,
            subject: Some(subj.into()), text: text.into(), metadata: serde_json::json!({}),
        };
        let a = mk("a", "ledger", "ledger contract draft", 1);
        let b = mk("b", "ledger", "ledger contract draft", 30);
        w.upsert(&[a.clone(), b.clone()]).await.unwrap();

        // Same vector for both — recency should break the tie in favor of `a`.
        let v: Vec<f32> = (0..1536).map(|i| (i as f32) / 1536.0).collect();
        w.upsert_vector(&a.id, &v).await.unwrap();
        w.upsert_vector(&b.id, &v).await.unwrap();

        let reader = IndexReader::new(&idx);
        let q = Query::simple("ledger contract", 5);
        let hits = reader.hybrid_search(&q, &v).await.unwrap();
        assert_eq!(hits[0].item.external_id, "a", "recency breaks tie");
    }
}
```

- [ ] **Step 2: Verify failure**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::index::reader_hybrid_tests -- --nocapture
```
Expected: FAIL — `hybrid_search` undefined.

- [ ] **Step 3: Implement hybrid**

Add to `impl<'a> IndexReader<'a>` in `src/openhuman/life_capture/index.rs`:

```rust
    /// Combines vector + keyword scores with a recency boost.
    /// score = 0.55 * vector_norm + 0.35 * keyword_norm + 0.10 * recency_norm
    pub async fn hybrid_search(
        &self,
        q: &crate::openhuman::life_capture::types::Query,
        query_vector: &[f32],
    ) -> sqlx::Result<Vec<Hit>> {
        // Pull more than k from each leg and re-rank in app code.
        let oversample = (q.k * 3).max(20);
        let kw = self.keyword_search(&q.text, oversample).await?;
        let vc = self.vector_search(query_vector, oversample).await?;

        use std::collections::HashMap;
        let mut by_id: HashMap<uuid::Uuid, (Hit, f32, f32)> = HashMap::new();

        let max_kw = kw.iter().map(|h| h.score).fold(f32::MIN, f32::max).max(1e-6);
        let max_vc = vc.iter().map(|h| h.score).fold(f32::MIN, f32::max).max(1e-6);

        for h in kw { let s = h.score / max_kw; by_id.insert(h.item.id, (h, s, 0.0)); }
        for h in vc {
            let s = h.score / max_vc;
            by_id.entry(h.item.id)
                .and_modify(|(_, _, vs)| { *vs = s; })
                .or_insert_with(|| (h.clone(), 0.0, s));
        }

        let now = chrono::Utc::now().timestamp();
        let mut out: Vec<Hit> = by_id.into_values().map(|(mut hit, kw_n, vc_n)| {
            let age_days = ((now - hit.item.ts.timestamp()).max(0) as f32) / 86400.0;
            let recency = (-age_days / 30.0).exp(); // half-life ~ 21 days
            let score = 0.55 * vc_n + 0.35 * kw_n + 0.10 * recency;
            hit.score = score;
            hit
        }).collect();

        out.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        out.truncate(q.k);
        Ok(out)
    }
```

- [ ] **Step 4: Run, verify pass, commit**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::index::reader_hybrid_tests -- --nocapture
```
Expected: PASS.

```bash
git add src/openhuman/life_capture/index.rs
git commit -m "feat(life_capture): hybrid_search combining vector + keyword + recency"
```

---

## Task F12: Controller schema and RPC handlers

**Files:**
- Modify: `src/openhuman/life_capture/schemas.rs`
- Modify: `src/openhuman/life_capture/rpc.rs`
- Modify: `src/openhuman/life_capture/mod.rs` (re-exports if needed)

Per the repo's "controller-only exposure" rule, the only public surface is through controller schemas registered into `core_server::dispatch`.

- [ ] **Step 1: Write controller schemas**

Replace `src/openhuman/life_capture/schemas.rs`:

```rust
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

pub fn life_capture_schema() -> ControllerSchema {
    ControllerSchema {
        name: "life_capture".into(),
        description: "Local personal index — search, stats, status.".into(),
        methods: vec![
            crate::core::MethodSchema {
                name: "get_stats".into(),
                description: "Return total item counts by source and last-ingest timestamp.".into(),
                params: vec![],
                returns: TypeSchema::object("IndexStats", vec![
                    FieldSchema::new("total_items", TypeSchema::U64),
                    FieldSchema::new("by_source", TypeSchema::array(TypeSchema::object("SourceCount", vec![
                        FieldSchema::new("source", TypeSchema::String),
                        FieldSchema::new("count", TypeSchema::U64),
                    ]))),
                    FieldSchema::new("last_ingest_ts", TypeSchema::optional(TypeSchema::DateTime)),
                ]),
            },
            crate::core::MethodSchema {
                name: "search".into(),
                description: "Hybrid search over the personal index.".into(),
                params: vec![
                    FieldSchema::new("text", TypeSchema::String),
                    FieldSchema::new("k", TypeSchema::U64),
                ],
                returns: TypeSchema::array(TypeSchema::object("Hit", vec![
                    FieldSchema::new("item_id", TypeSchema::String),
                    FieldSchema::new("score", TypeSchema::F32),
                    FieldSchema::new("snippet", TypeSchema::String),
                    FieldSchema::new("source", TypeSchema::String),
                    FieldSchema::new("subject", TypeSchema::optional(TypeSchema::String)),
                    FieldSchema::new("ts", TypeSchema::DateTime),
                ])),
            },
        ],
    }
}
```

(Match the actual constructor / API of `ControllerSchema` and `TypeSchema` as defined in `src/core/mod.rs`. If those types use builder methods rather than struct literals, adapt — the goal is the same: register two methods with their typed params and returns.)

- [ ] **Step 2: Implement the handlers**

Replace `src/openhuman/life_capture/rpc.rs`:

```rust
use crate::core::RpcOutcome;
use crate::openhuman::life_capture::{IndexReader, PersonalIndex, Query};
use serde_json::Value;

pub async fn handle_get_stats(idx: &PersonalIndex) -> RpcOutcome<Value> {
    let total: (i64,) = match sqlx::query_as("SELECT count(*) FROM items").fetch_one(&idx.pool).await {
        Ok(v) => v, Err(e) => return RpcOutcome::error(format!("count: {e}")),
    };
    let by_source: Vec<(String, i64)> = match sqlx::query_as(
        "SELECT source, count(*) FROM items GROUP BY source"
    ).fetch_all(&idx.pool).await {
        Ok(v) => v, Err(e) => return RpcOutcome::error(format!("by_source: {e}")),
    };
    let last_ts: Option<(i64,)> = sqlx::query_as("SELECT max(ts) FROM items")
        .fetch_optional(&idx.pool).await.ok().flatten();
    RpcOutcome::ok(serde_json::json!({
        "total_items": total.0,
        "by_source": by_source.iter().map(|(s,c)| serde_json::json!({"source": s, "count": c})).collect::<Vec<_>>(),
        "last_ingest_ts": last_ts.and_then(|(t,)| chrono::DateTime::from_timestamp(t, 0)),
    }))
}

pub async fn handle_search(
    idx: &PersonalIndex,
    embedder: &dyn crate::openhuman::life_capture::Embedder,
    text: String,
    k: usize,
) -> RpcOutcome<Value> {
    let vec = match embedder.embed_batch(&[text.as_str()]).await {
        Ok(mut v) => v.remove(0),
        Err(e) => return RpcOutcome::error(format!("embed: {e}")),
    };
    let reader = IndexReader::new(idx);
    let q = Query::simple(text, k);
    match reader.hybrid_search(&q, &vec).await {
        Ok(hits) => RpcOutcome::ok(serde_json::json!(hits.iter().map(|h| serde_json::json!({
            "item_id": h.item.id.to_string(),
            "score": h.score,
            "snippet": h.snippet,
            "source": serde_json::to_value(&h.item.source).unwrap(),
            "subject": h.item.subject,
            "ts": h.item.ts,
        })).collect::<Vec<_>>())),
        Err(e) => RpcOutcome::error(format!("hybrid_search: {e}")),
    }
}
```

(Adapt `RpcOutcome::ok` / `RpcOutcome::error` to the actual constructors used in `src/core/`.)

- [ ] **Step 3: Register the controller in `core_server::dispatch`**

Locate where other controllers register. Add:

```rust
dispatch.register_schema(crate::openhuman::life_capture::schemas::life_capture_schema());
dispatch.register_handler("life_capture.get_stats", |ctx, _params| async move {
    crate::openhuman::life_capture::rpc::handle_get_stats(&ctx.life_capture_index).await
});
dispatch.register_handler("life_capture.search", |ctx, params| async move {
    let text: String = params.get_required("text")?;
    let k: usize = params.get_optional("k").unwrap_or(10);
    crate::openhuman::life_capture::rpc::handle_search(
        &ctx.life_capture_index, &*ctx.embedder, text, k
    ).await
});
```

(The exact registration API depends on existing `dispatch` patterns — match what's already in use for other controllers, e.g. `cron`. The intent is: schema + 2 handlers wired up.)

- [ ] **Step 4: Compile + commit**

```bash
cargo check --manifest-path Cargo.toml
```
Expected: clean (will fail if `ctx.life_capture_index()` / `ctx.embedder()` accessors don't exist yet — add them to the existing context type, returning shared instances stored on the dispatch context).

```bash
git add -A
git commit -m "feat(life_capture): controller schema + RPC handlers (get_stats, search)"
```

---

## Task F13: End-to-end integration test

**Files:**
- Create: `src/openhuman/life_capture/tests/e2e.rs`
- Create: `src/openhuman/life_capture/tests/mod.rs`

- [ ] **Step 1: Create the test module**

`src/openhuman/life_capture/tests/mod.rs`:

```rust
mod e2e;
```

`src/openhuman/life_capture/tests/e2e.rs`:

```rust
use crate::openhuman::life_capture::*;
use async_trait::async_trait;
use chrono::Utc;
use uuid::Uuid;

/// Deterministic embedder for tests: hash the text into a sparse vector.
struct FakeEmbedder;

#[async_trait]
impl Embedder for FakeEmbedder {
    async fn embed_batch(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| {
            let mut v = vec![0.0_f32; 1536];
            for (i, b) in t.as_bytes().iter().enumerate().take(64) {
                v[(*b as usize) % 1536] += 1.0 / (1.0 + i as f32);
            }
            v
        }).collect())
    }
    fn dim(&self) -> usize { 1536 }
}

#[tokio::test]
async fn ingest_then_retrieve_with_redaction_and_quote_strip() {
    let idx = PersonalIndex::open_in_memory().await.unwrap();
    let writer = IndexWriter::new(&idx);
    let embedder = FakeEmbedder;

    let raw = "The Ledger contract draft is ready, sarah@example.com signed.\n\n\
               On Mon, Apr 21, 2026 at 9:14 AM, Sarah <sarah@x> wrote:\n\
               > earlier text we don't want indexed";
    let cleaned = redact::redact(&quote_strip::strip_quoted_reply(raw));
    assert!(!cleaned.contains("earlier text"));
    assert!(cleaned.contains("<EMAIL>"));

    let item = Item {
        id: Uuid::new_v4(), source: Source::Gmail,
        external_id: "msg-1".into(), ts: Utc::now(),
        author: None, subject: Some("Ledger contract".into()),
        text: cleaned, metadata: serde_json::json!({}),
    };
    writer.upsert(&[item.clone()]).await.unwrap();

    let vecs = embedder.embed_batch(&[item.text.as_str()]).await.unwrap();
    writer.upsert_vector(&item.id, &vecs[0]).await.unwrap();

    let reader = IndexReader::new(&idx);
    let q = Query::simple("ledger contract", 5);
    let qvec = embedder.embed_batch(&[&q.text]).await.unwrap().remove(0);
    let hits = reader.hybrid_search(&q, &qvec).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].item.external_id, "msg-1");
    assert!(!hits[0].item.text.contains("earlier text"), "quote was stripped pre-index");
}
```

- [ ] **Step 2: Wire the test module into `mod.rs`**

In `src/openhuman/life_capture/mod.rs`, add at the bottom (gated):

```rust
#[cfg(test)]
mod tests;
```

- [ ] **Step 3: Run the test**

```bash
cargo test --manifest-path Cargo.toml -p openhuman_core life_capture::tests -- --nocapture
```
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/openhuman/life_capture/tests/ src/openhuman/life_capture/mod.rs
git commit -m "test(life_capture): end-to-end (redact → quote-strip → embed → upsert → search)"
```

---

## Task F14: Wire into core context + first manual smoke

**Files:**
- Modify: the dispatch context type that owns long-lived services (where `MemoryClient` lives today)
- Modify: app data dir resolution to include `personal_index.db`

- [ ] **Step 1: Add the `PersonalIndex` to the dispatch context**

Locate the struct that holds `MemoryClient` (likely `core_server::Context` or similar). Add a field:

```rust
pub life_capture_index: std::sync::Arc<crate::openhuman::life_capture::PersonalIndex>,
pub embedder: std::sync::Arc<dyn crate::openhuman::life_capture::Embedder>,
```

- [ ] **Step 2: Construct on startup**

Where the context is built (server startup), add:

```rust
let app_data_dir = crate::openhuman::config::resolve_dirs()?.data_dir;
let index_path = app_data_dir.join("personal_index.db");
let life_capture_index = std::sync::Arc::new(
    crate::openhuman::life_capture::PersonalIndex::open(&index_path).await?
);

// Default embedder: hosted OpenAI. Reads key from existing config layer.
let cfg = crate::openhuman::config::current();
let embedder: std::sync::Arc<dyn crate::openhuman::life_capture::Embedder> =
    std::sync::Arc::new(crate::openhuman::life_capture::HostedEmbedder::new(
        cfg.embeddings.base_url.clone(),
        cfg.embeddings.api_key.clone(),
        cfg.embeddings.model.clone(),
    ));
```

(If the config struct doesn't yet have an `embeddings` section, add one to `src/openhuman/config/schema/types.rs` with sensible defaults: `base_url = "https://api.openai.com/v1"`, `model = "text-embedding-3-small"`, `api_key` from env `OPENHUMAN_EMBEDDINGS_KEY` or falls back to `OPENAI_API_KEY`. Update `load.rs` to apply env overrides.)

- [ ] **Step 3: Build and run a manual smoke**

```bash
cargo build --manifest-path Cargo.toml --bin openhuman
OPENAI_API_KEY=sk-... ./target/debug/openhuman serve &
SERVER_PID=$!
sleep 2

# Hit the controller via CLI (using the existing controller-call CLI surface).
./target/debug/openhuman call life_capture.get_stats
# Expected: {"total_items": 0, "by_source": [], "last_ingest_ts": null}

kill $SERVER_PID
```
Expected: clean startup, `personal_index.db` created in the app data dir, `get_stats` returns zeros.

- [ ] **Step 4: Verify the DB file**

```bash
ls -la "$(./target/debug/openhuman config get data_dir)/personal_index.db"
sqlite3 "$(./target/debug/openhuman config get data_dir)/personal_index.db" ".tables"
```
Expected: file exists; `.tables` lists `items`, `items_fts`, `item_vectors`, `_life_capture_migrations`.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(life_capture): wire PersonalIndex + Embedder into core dispatch context"
```

---

## Acceptance for Foundation milestone

- [ ] All 14 tasks merged onto a feature branch (`feat/life-capture-foundation`)
- [ ] `cargo test life_capture` passes (every sub-module test + e2e)
- [ ] On startup, `personal_index.db` is created in the app data dir with the expected schema
- [ ] `life_capture.get_stats` and `life_capture.search` callable via the existing controller dispatch (CLI and JSON-RPC)
- [ ] No new files added at `src/openhuman/` root (per repo rule: new functionality lives in a subdirectory)
- [ ] No domain logic in `core_server/` — all of it lives in `life_capture/`
- [ ] PR opened, CI green, merged into `main`

Once this milestone lands, write **Plan #2 — Ingestors** (Gmail, Calendar, iMessage, scheduler) against this foundation.
