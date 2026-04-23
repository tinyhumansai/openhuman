-- Chronicle S0/S1 event store (A3).
--
-- chronicle_events stores deduped + parsed focus/capture events. Each row is
-- a single moment of user context: which app/element was focused, what text
-- was visible (PII-redacted), optional URL for browser classes. Later slices
-- (A4 bucketing, A6 daily reducer, A8 entity extraction) read from here.
--
-- chronicle_watermark is a resumable cursor table so dispatchers can pick up
-- where they left off after a restart. Keyed by source name so multiple
-- dispatchers (e.g. screen focus, calendar sync, inbox tick) coexist.
CREATE TABLE IF NOT EXISTS chronicle_events (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_ms            INTEGER NOT NULL,           -- unix milliseconds
    focused_app      TEXT NOT NULL,              -- bundle id or exe name
    focused_element  TEXT,                       -- accessibility role + label, nullable
    visible_text     TEXT,                       -- PII-redacted body
    url              TEXT,                       -- only set for browser-class apps
    -- unix MILLISECONDS (matches ts_ms). Was seconds; kept ms to avoid a unit
    -- mismatch with ts_ms during downstream reductions.
    created_at       INTEGER NOT NULL DEFAULT (CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER))
);

-- Composite (ts_ms DESC, id DESC) matches the list_recent ORDER BY exactly so
-- the query can walk the index without a filesort on ts_ms ties.
CREATE INDEX IF NOT EXISTS chronicle_events_ts_id_idx     ON chronicle_events(ts_ms DESC, id DESC);
CREATE INDEX IF NOT EXISTS chronicle_events_app_ts_idx    ON chronicle_events(focused_app, ts_ms DESC);

CREATE TABLE IF NOT EXISTS chronicle_watermark (
    source      TEXT PRIMARY KEY,
    last_ts_ms  INTEGER NOT NULL
);
