use anyhow::{Context, Result};
use rusqlite::Connection;

pub(crate) fn init_run_ledger_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_runs (
            id                 TEXT PRIMARY KEY,
            kind               TEXT NOT NULL,
            parent_run_id      TEXT,
            parent_thread_id   TEXT,
            agent_id           TEXT,
            status             TEXT NOT NULL,
            prompt_ref         TEXT,
            worker_thread_id   TEXT,
            task_board_id      TEXT,
            task_card_id       TEXT,
            checkpoint_path    TEXT,
            checkpoint_json    TEXT,
            summary            TEXT,
            error              TEXT,
            metadata_json      TEXT NOT NULL DEFAULT '{}',
            started_at         TEXT NOT NULL,
            updated_at         TEXT NOT NULL,
            completed_at       TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_agent_runs_status ON agent_runs(status);
        CREATE INDEX IF NOT EXISTS idx_agent_runs_kind ON agent_runs(kind);
        CREATE INDEX IF NOT EXISTS idx_agent_runs_parent ON agent_runs(parent_run_id);
        CREATE INDEX IF NOT EXISTS idx_agent_runs_thread ON agent_runs(parent_thread_id);
        CREATE INDEX IF NOT EXISTS idx_agent_runs_updated ON agent_runs(updated_at);
        CREATE INDEX IF NOT EXISTS idx_agent_runs_worker_thread ON agent_runs(worker_thread_id);

        CREATE TABLE IF NOT EXISTS workflow_runs (
            id                 TEXT PRIMARY KEY,
            definition_id      TEXT NOT NULL,
            parent_thread_id   TEXT,
            input_json         TEXT NOT NULL DEFAULT '{}',
            phase_states_json  TEXT NOT NULL DEFAULT '{}',
            child_run_ids_json TEXT NOT NULL DEFAULT '[]',
            status             TEXT NOT NULL,
            summary            TEXT,
            started_at         TEXT NOT NULL,
            updated_at         TEXT NOT NULL,
            completed_at       TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_runs_definition ON workflow_runs(definition_id);
        CREATE INDEX IF NOT EXISTS idx_workflow_runs_status ON workflow_runs(status);
        CREATE INDEX IF NOT EXISTS idx_workflow_runs_thread ON workflow_runs(parent_thread_id);

        CREATE TABLE IF NOT EXISTS run_events (
            run_id      TEXT NOT NULL,
            sequence    INTEGER NOT NULL,
            event_type  TEXT NOT NULL,
            payload_json TEXT NOT NULL DEFAULT '{}',
            timestamp   TEXT NOT NULL,
            PRIMARY KEY (run_id, sequence)
        );
        CREATE INDEX IF NOT EXISTS idx_run_events_timestamp ON run_events(timestamp);

        CREATE TABLE IF NOT EXISTS run_telemetry (
            run_id              TEXT PRIMARY KEY,
            input_tokens        INTEGER NOT NULL DEFAULT 0,
            output_tokens       INTEGER NOT NULL DEFAULT 0,
            cached_input_tokens INTEGER NOT NULL DEFAULT 0,
            cost_usd            REAL NOT NULL DEFAULT 0.0,
            elapsed_ms          INTEGER,
            tool_count          INTEGER NOT NULL DEFAULT 0,
            model               TEXT,
            provider            TEXT,
            error               TEXT,
            updated_at          TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS agent_teams (
            id                 TEXT PRIMARY KEY,
            parent_thread_id   TEXT,
            lead_agent_id      TEXT NOT NULL,
            status             TEXT NOT NULL,
            summary            TEXT,
            created_at         TEXT NOT NULL,
            updated_at         TEXT NOT NULL,
            closed_at          TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_agent_teams_thread ON agent_teams(parent_thread_id);
        CREATE INDEX IF NOT EXISTS idx_agent_teams_status ON agent_teams(status);

        CREATE TABLE IF NOT EXISTS agent_team_members (
            id                 TEXT PRIMARY KEY,
            team_id            TEXT NOT NULL,
            name               TEXT NOT NULL,
            agent_id           TEXT,
            member_status      TEXT NOT NULL,
            current_task_id    TEXT,
            worker_thread_id   TEXT,
            run_id             TEXT,
            created_at         TEXT NOT NULL,
            updated_at         TEXT NOT NULL,
            UNIQUE(team_id, name)
        );
        CREATE INDEX IF NOT EXISTS idx_agent_team_members_team ON agent_team_members(team_id);

        CREATE TABLE IF NOT EXISTS agent_team_tasks (
            id                  TEXT PRIMARY KEY,
            team_id             TEXT NOT NULL,
            title               TEXT NOT NULL,
            objective           TEXT,
            status              TEXT NOT NULL,
            owner_member_id     TEXT,
            claimed_by_member_id TEXT,
            claim_token         TEXT,
            depends_on_json     TEXT NOT NULL DEFAULT '[]',
            gate_status         TEXT NOT NULL DEFAULT 'pending',
            gate_reason         TEXT,
            evidence_json       TEXT NOT NULL DEFAULT '[]',
            source_run_id       TEXT,
            order_index         INTEGER NOT NULL DEFAULT 0,
            created_at          TEXT NOT NULL,
            updated_at          TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_agent_team_tasks_team ON agent_team_tasks(team_id);
        CREATE INDEX IF NOT EXISTS idx_agent_team_tasks_status ON agent_team_tasks(status);
        CREATE INDEX IF NOT EXISTS idx_agent_team_tasks_claimed ON agent_team_tasks(claimed_by_member_id);",
    )
    .context("failed to initialize run ledger schema")
}
