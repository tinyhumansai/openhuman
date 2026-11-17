// A layout-v1 profile as the pre-storage releases left it: per-domain SQLite
// files, a cost ledger, a classic session transcript and an auth-profiles
// file, all fictional. The table shapes are the ones the importers read (the
// same DDL as tests/storage_desktop_upgrade_e2e.rs); the tinyflows stores are
// the committed fixture written by the retired tinyflows-sqlite writers.

import fsp from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { sqliteFile } from "./lib.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, "..", "..");
export const TINYFLOWS_FIXTURE = path.join(
  REPO,
  "crates/openhuman-core/src/storage/import/fixtures/tinyflows",
);

/** What the seeded profile holds, so checks read against the same numbers. */
export const LEGACY_EXPECT = {
  approvals: ["still-pending", "already-approved"],
  flowTrust: "flow-1",
  devices: ["chan-a", "chan-b"],
  notifications: ["n-1"],
  taskSources: ["src-1"],
  costs: ["storscen-cost-1", "storscen-cost-2", "storscen-cost-3"],
  provider: "storscen-legacy-provider",
  // The import markers (`storage.import_state`, `storage_import/<domain>@<digest>`).
  markerDomains: ["approval", "devices", "notifications", "task_sources", "cron", "flows", "sessions"],
};

async function copyTree(from, to) {
  await fsp.mkdir(to, { recursive: true });
  for (const e of await fsp.readdir(from, { withFileTypes: true })) {
    const s = path.join(from, e.name);
    const d = path.join(to, e.name);
    if (e.isDirectory()) await copyTree(s, d);
    else await fsp.copyFile(s, d);
  }
}

async function write(file, text) {
  await fsp.mkdir(path.dirname(file), { recursive: true });
  await fsp.writeFile(file, text);
}

/** The cron jobs and flows of the committed tinyflows fixture. */
export async function tinyflowsExpected() {
  return JSON.parse(
    await fsp.readFile(path.join(TINYFLOWS_FIXTURE, "expected.json"), "utf8"),
  );
}

/**
 * Seed `users/<userId>/` under `oh` (`$HOME/.openhuman`) as a v1 profile and
 * make it the active user. `providerToken` goes into the legacy
 * auth-profiles.json in plaintext, as the old file kept it.
 */
export async function seedV1Profile({ oh, userId, baseConfig, providerToken }) {
  const profile = path.join(oh, "users", userId);
  const ws = path.join(profile, "workspace");
  await write(path.join(oh, "active_user.toml"), `user_id = "${userId}"\n`);
  await write(path.join(profile, "config.toml"), baseConfig);

  await copyTree(path.join(TINYFLOWS_FIXTURE, "workspace"), ws);

  sqliteFile(
    path.join(ws, "approval/approval.db"),
    `CREATE TABLE pending_approvals (
       request_id TEXT PRIMARY KEY, tool_name TEXT NOT NULL,
       action_summary TEXT NOT NULL, args_redacted TEXT NOT NULL,
       session_id TEXT NOT NULL, created_at TEXT NOT NULL, expires_at TEXT,
       decided_at TEXT, decision TEXT, executed_at TEXT, execution_outcome TEXT,
       execution_error TEXT, source_context TEXT, tool_call_id TEXT, agent_id TEXT);
     CREATE TABLE flow_tool_trust (
       flow_id TEXT NOT NULL, tool_name TEXT NOT NULL, created_at TEXT NOT NULL,
       PRIMARY KEY (flow_id, tool_name));
     INSERT INTO pending_approvals
       (request_id, tool_name, action_summary, args_redacted, session_id, created_at, expires_at)
     VALUES ('still-pending', 'shell', 'run ls', '{"cmd":"ls"}', 'sess',
             '2026-01-01T00:00:00+00:00', '2099-01-01T00:00:00+00:00');
     INSERT INTO pending_approvals
       (request_id, tool_name, action_summary, args_redacted, session_id, created_at,
        expires_at, decided_at, decision)
     VALUES ('already-approved', 'shell', 'run pwd', '{}', 'sess',
             '2026-01-01T00:00:00+00:00', '2099-01-01T00:00:00+00:00',
             '2026-01-01T00:01:00+00:00', 'approve_once');
     INSERT INTO flow_tool_trust (flow_id, tool_name, created_at)
     VALUES ('flow-1', 'shell', '2026-01-01T00:00:00+00:00');`,
  );
  sqliteFile(
    path.join(ws, "devices/devices.db"),
    `CREATE TABLE paired_devices (
       channel_id TEXT PRIMARY KEY, label TEXT NOT NULL, device_pubkey TEXT NOT NULL,
       core_session_token_hash TEXT NOT NULL, shared_secret_encrypted BLOB,
       created_at TEXT NOT NULL, last_seen_at TEXT, revoked INTEGER NOT NULL DEFAULT 0);
     INSERT INTO paired_devices
       (channel_id, label, device_pubkey, core_session_token_hash, created_at, revoked)
     VALUES ('chan-a', 'Phone', 'pk', 'hash', '2026-01-01T00:00:00+00:00', 0),
            ('chan-b', 'Old laptop', 'pk', 'hash', '2026-01-01T00:00:00+00:00', 1);`,
  );
  sqliteFile(
    path.join(ws, "notifications/notifications.db"),
    `CREATE TABLE integration_notifications (
       id TEXT PRIMARY KEY, provider TEXT NOT NULL, account_id TEXT, title TEXT NOT NULL,
       body TEXT NOT NULL, raw_payload TEXT NOT NULL, importance_score REAL,
       triage_action TEXT, triage_reason TEXT, status TEXT NOT NULL DEFAULT 'unread',
       received_at TEXT NOT NULL, scored_at TEXT);
     CREATE TABLE notification_settings (
       provider TEXT PRIMARY KEY, enabled INTEGER NOT NULL DEFAULT 1,
       importance_threshold REAL NOT NULL DEFAULT 0.0,
       route_to_orchestrator INTEGER NOT NULL DEFAULT 1);
     CREATE TABLE core_notifications (
       id TEXT PRIMARY KEY, payload TEXT NOT NULL, timestamp_ms INTEGER NOT NULL,
       read INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);
     INSERT INTO integration_notifications
       (id, provider, account_id, title, body, raw_payload, received_at)
     VALUES ('n-1', 'gmail', 'acct', 'Hello', 'World', '{}', '2026-01-01T00:00:00+00:00');
     INSERT INTO notification_settings VALUES ('gmail', 1, 0.5, 1);
     INSERT INTO core_notifications VALUES
       ('core-1', '{"id":"core-1","title":"cron done"}', 1767225600000, 0,
        '2026-01-01T00:00:00+00:00');`,
  );
  sqliteFile(
    path.join(ws, "task_sources/sources.db"),
    `CREATE TABLE task_sources (
       id TEXT PRIMARY KEY, provider TEXT NOT NULL, connection_id TEXT, name TEXT,
       enabled INTEGER NOT NULL DEFAULT 1, filter TEXT NOT NULL, interval_secs INTEGER NOT NULL,
       target TEXT NOT NULL, max_tasks_per_fetch INTEGER NOT NULL, created_at TEXT NOT NULL,
       last_fetch_at TEXT, last_status TEXT);
     CREATE TABLE ingested_tasks (
       source_id TEXT NOT NULL, external_id TEXT NOT NULL, content_hash TEXT NOT NULL,
       title TEXT, payload TEXT, ingested_at TEXT NOT NULL, card_id TEXT,
       PRIMARY KEY (source_id, external_id));
     INSERT INTO task_sources
       (id, provider, filter, interval_secs, target, max_tasks_per_fetch, created_at)
     VALUES ('src-1', 'github', '{"provider":"github"}', 600, '"todo_only"', 5,
             '2026-01-01T00:00:00+00:00');
     PRAGMA user_version = 0;
     INSERT INTO ingested_tasks (source_id, external_id, content_hash, ingested_at)
     VALUES ('src-1', 'issue-9', 'abc', '2026-01-01T00:00:00+00:00');`,
  );
  sqliteFile(
    path.join(ws, "session_db/sessions.db"),
    `CREATE TABLE sessions (
       id TEXT PRIMARY KEY, agent_definition_id TEXT NOT NULL,
       agent_definition_name TEXT NOT NULL, session_key TEXT NOT NULL,
       parent_session_id TEXT, thread_id TEXT, source_channel TEXT,
       status TEXT NOT NULL DEFAULT 'running', model TEXT,
       turn_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL DEFAULT 0,
       output_tokens INTEGER NOT NULL DEFAULT 0, cached_input_tokens INTEGER NOT NULL DEFAULT 0,
       cost_usd REAL NOT NULL DEFAULT 0.0, transcript_path TEXT,
       started_at TEXT NOT NULL, ended_at TEXT);
     INSERT INTO sessions (id, agent_definition_id, agent_definition_name, session_key, started_at)
     VALUES ('sess-1', 'orchestrator', 'Orchestrator', 'thread-1', '2026-01-01T00:00:00Z');`,
  );
  // A classic transcript: the `_meta` line, then one message. An unknown stem
  // falls back to the default agent.
  await write(
    path.join(ws, "session_raw/storscen-legacy-thread.jsonl"),
    [
      JSON.stringify({
        _meta: {
          version: 1,
          agent: "orchestrator",
          dispatcher: "native",
          created: "2026-01-01T00:00:00Z",
          updated: "2026-01-01T00:00:00Z",
          turn_count: 1,
          input_tokens: 0,
          output_tokens: 0,
          cached_input_tokens: 0,
          charged_amount_usd: 0,
          agent_id: "orchestrator",
          thread_id: "storscen-legacy-thread",
        },
      }),
      JSON.stringify({ role: "user", content: "hello from the old layout" }),
      "",
    ].join("\n"),
  );

  // The cost ledger, stamped recently so a 30-day usage log lists it.
  const now = Date.now();
  await write(
    path.join(ws, "state/costs.jsonl"),
    LEGACY_EXPECT.costs
      .map((id, i) =>
        JSON.stringify({
          id,
          session_id: "sess-1",
          usage: {
            model: "storscen-fixture-model",
            input_tokens: 10 + i,
            output_tokens: 5,
            total_tokens: 15 + i,
            cost_usd: 0.001 * (i + 1),
            timestamp: new Date(now - (i + 1) * 3_600_000).toISOString(),
          },
        }),
      )
      .join("\n") + "\n",
  );

  // The credential file sits beside config.toml, token in plaintext as the
  // old file could hold it (adopted, then encrypted, on first read).
  const stamp = "2026-01-01T00:00:00Z";
  await write(
    path.join(profile, "auth-profiles.json"),
    JSON.stringify(
      {
        schema_version: 1,
        updated_at: stamp,
        active_profiles: { [LEGACY_EXPECT.provider]: `${LEGACY_EXPECT.provider}:default` },
        profiles: {
          [`${LEGACY_EXPECT.provider}:default`]: {
            provider: LEGACY_EXPECT.provider,
            profile_name: "default",
            kind: "token",
            token: providerToken,
            created_at: stamp,
            updated_at: stamp,
          },
        },
      },
      null,
      2,
    ),
  );
  return { profile, workspace: ws };
}
