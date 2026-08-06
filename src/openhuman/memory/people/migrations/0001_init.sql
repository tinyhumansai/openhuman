-- People module schema.
--
-- `people` holds one row per resolved person. `handle_aliases` holds all
-- known (kind, canonical_value) handles that map to that person; the
-- resolver is a lookup on `(kind, value)` → `person_id`.
--
-- `interactions` records observed exchanges for scoring. Single-user v1;
-- each row is attributed to (local-user, person_id).

CREATE TABLE IF NOT EXISTS people (
    id             TEXT PRIMARY KEY,            -- uuid
    display_name   TEXT,
    primary_email  TEXT,
    primary_phone  TEXT,
    created_at     INTEGER NOT NULL,            -- unix seconds
    updated_at     INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS handle_aliases (
    kind       TEXT NOT NULL,                   -- 'imessage' | 'email' | 'display_name'
    value      TEXT NOT NULL,                   -- canonicalized (lowercase / trimmed)
    person_id  TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (kind, value)
);

CREATE INDEX IF NOT EXISTS handle_aliases_person_idx ON handle_aliases(person_id);

CREATE TABLE IF NOT EXISTS interactions (
    person_id   TEXT NOT NULL REFERENCES people(id) ON DELETE CASCADE,
    ts          INTEGER NOT NULL,               -- unix seconds
    is_outbound INTEGER NOT NULL,               -- 1 = user sent, 0 = received
    length      INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS interactions_person_idx ON interactions(person_id, ts DESC);
CREATE INDEX IF NOT EXISTS interactions_ts_idx     ON interactions(ts DESC);
