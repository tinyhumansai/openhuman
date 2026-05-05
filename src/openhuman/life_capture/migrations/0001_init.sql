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
