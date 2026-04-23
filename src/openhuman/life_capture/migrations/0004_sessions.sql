-- Chronicle session manager (A4).
--
-- Sessions group chronicle_events into coherent work blocks bounded by
-- idle gaps, sustained app switches, or a hard 2h cap. A session has a
-- start_ts_ms (first event inside the block), a close_ts_ms (last event
-- observed before the boundary fired), and a boundary_reason explaining
-- why it ended. Sessions are "closed" when written — open sessions are
-- kept in-memory by the manager until a boundary condition fires.
--
-- chronicle_minute_buckets is a 1-minute rollup of events per session:
-- one row per (session_id, bucket_ts_ms) with the dominant app inside
-- that minute. Downstream reducers (A6) consume buckets, not raw events.
--
-- A4 keeps no RPC surface — these tables are internal until A6 needs
-- them. See src/openhuman/life_capture/chronicle/sessions/ for the
-- manager that writes here.
CREATE TABLE IF NOT EXISTS chronicle_sessions (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    start_ts_ms      INTEGER NOT NULL,           -- ts_ms of first event in session
    close_ts_ms      INTEGER NOT NULL,           -- ts_ms of last event observed
    boundary_reason  TEXT NOT NULL,              -- "idle_5m" | "app_switch_3m" | "max_2h"
    primary_app      TEXT NOT NULL,              -- app with most events in session
    event_count      INTEGER NOT NULL,           -- raw chronicle_events rolled up
    created_at       INTEGER NOT NULL DEFAULT (CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER))
);

-- Timeline queries want sessions newest-first on close_ts_ms.
CREATE INDEX IF NOT EXISTS chronicle_sessions_close_idx ON chronicle_sessions(close_ts_ms DESC, id DESC);
-- Daily reducer (A6) filters by start day → range scan on start_ts_ms.
CREATE INDEX IF NOT EXISTS chronicle_sessions_start_idx ON chronicle_sessions(start_ts_ms);

CREATE TABLE IF NOT EXISTS chronicle_minute_buckets (
    session_id       INTEGER NOT NULL REFERENCES chronicle_sessions(id) ON DELETE CASCADE,
    bucket_ts_ms     INTEGER NOT NULL,           -- floor(ts_ms / 60_000) * 60_000
    focused_app      TEXT NOT NULL,              -- dominant app during this minute
    event_count      INTEGER NOT NULL,
    PRIMARY KEY (session_id, bucket_ts_ms)
);

CREATE INDEX IF NOT EXISTS chronicle_minute_buckets_ts_idx
    ON chronicle_minute_buckets(bucket_ts_ms DESC);
