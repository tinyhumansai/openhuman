//! Desktop upgrade: a workspace written by the pre-storage releases, with every
//! per-domain database and JSON/JSONL file the importers read, is brought onto
//! the storage backend losslessly, and the legacy files are left byte-for-byte
//! as they were.
//!
//! Earlier tests proved each domain on its own (approvals, devices,
//! notifications, task sources, flows/cron, sessions, tinymcp, connectors,
//! x402, tinybox, tinyjuice). This one builds the whole workspace at once, runs
//! the host's import entry points in the order the host runs them, and checks
//! the result as a user upgrading would see it:
//!
//! * every record of every legacy store is in the backend (counts match what
//!   was written, ids spot-checked);
//! * a second boot imports nothing and resurrects nothing;
//! * no file of the workspace was modified, created or removed.
//!
//! Its own binary: it installs a storage backend into the process-wide slot and
//! sets the keyring master key. Each driver is its own case
//! (`support/storage_drivers.rs`), so the MongoDB case runs in the
//! `storage-mongodb` workflow against the replica set.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use openhuman_core::config::Config;
use openhuman_core::storage::import::{self, Outcome};
use openhuman_core::storage::{rusqlite, DocumentStoreExt, Scope, ScopedStorage, StorageBackend};
use sha2::Digest;
use tinyagents_session::transcript::{session_stem, SessionRef, TranscriptMessage, TranscriptMeta};
use tinyagents_session::turn_state::TurnState;
use tinystoragedrivers_core::{Capability, Query};

// The classic-transcript fixture writer, shared with the core's unit tests.
#[path = "../crates/openhuman-core/src/util/classic_transcript_fixture.rs"]
mod classic_transcript_fixture;

#[macro_use]
#[path = "support/storage_drivers.rs"]
mod storage_drivers;

use storage_drivers::Case;

/// The committed tinyflows workspace (written by the retired tinyflows-sqlite
/// writers): flows, revisions, runs, drafts, checkpoints and cron jobs.
fn tinyflows_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../openhuman-core/src/storage/import/fixtures/tinyflows/workspace")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// Every file under `root` with a digest of its bytes. SQLite's `-shm` index
/// and an empty `-wal` are connection bookkeeping a read-only open may create;
/// they hold no data and are left out.
fn tree(root: &Path) -> BTreeMap<String, String> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, root, out);
                continue;
            }
            let bytes = std::fs::read(&path).unwrap();
            let name = path.to_string_lossy();
            if name.ends_with("-shm") || (name.ends_with("-wal") && bytes.is_empty()) {
                continue;
            }
            out.insert(
                path.strip_prefix(root).unwrap().display().to_string(),
                hex::encode(sha2::Sha256::digest(bytes)),
            );
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn sqlite_file(workspace: &Path, relative: &str, ddl_and_rows: &str) {
    let db = workspace.join(relative);
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute_batch(ddl_and_rows)
        .unwrap();
}

fn write(workspace: &Path, relative: &str, text: &str) {
    let file = workspace.join(relative);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, text).unwrap();
}

/// Sessions as the classic layout kept them: a transcript, an in-flight turn
/// snapshot and a `sessions.db` row.
fn legacy_sessions(workspace: &Path) {
    let stem = session_stem(&SessionRef::scoped("thread-1", "orchestrator"));
    let raw = workspace.join("session_raw");
    std::fs::create_dir_all(&raw).unwrap();
    let meta = TranscriptMeta {
        agent_name: "orchestrator".into(),
        agent_id: Some("orchestrator".into()),
        agent_type: None,
        dispatcher: "native".into(),
        provider: None,
        model: None,
        created: "2026-01-01T00:00:00Z".into(),
        updated: "2026-01-01T00:00:00Z".into(),
        turn_count: 1,
        prefix_message_count: None,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: Some("thread-1".into()),
        task_id: None,
        session_id: None,
        parent_session_id: None,
    };
    classic_transcript_fixture::write_classic(
        &raw.join(format!("{stem}.jsonl")),
        &[TranscriptMessage::new("user", "hello from the old layout")],
        &meta,
    );
    let snapshot = TurnState::started("thread-1", "req-1", 4, "2026-01-01T00:00:00Z");
    let thread_dir = workspace
        .join("memory/conversations/turn_states")
        .join(hex::encode("thread-1"));
    std::fs::create_dir_all(&thread_dir).unwrap();
    std::fs::write(
        thread_dir.join(format!("{}.json", hex::encode("req-1"))),
        serde_json::to_vec(&snapshot).unwrap(),
    )
    .unwrap();
    sqlite_file(
        workspace,
        "session_db/sessions.db",
        "CREATE TABLE sessions (
            id TEXT PRIMARY KEY, agent_definition_id TEXT NOT NULL,
            agent_definition_name TEXT NOT NULL, session_key TEXT NOT NULL,
            parent_session_id TEXT, thread_id TEXT, source_channel TEXT,
            status TEXT NOT NULL DEFAULT 'running', model TEXT,
            turn_count INTEGER NOT NULL DEFAULT 0, input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0, cached_input_tokens INTEGER NOT NULL DEFAULT 0,
            cost_usd REAL NOT NULL DEFAULT 0.0, transcript_path TEXT,
            started_at TEXT NOT NULL, ended_at TEXT);
         INSERT INTO sessions (id, agent_definition_id, agent_definition_name, session_key, started_at)
         VALUES ('sess-1', 'orchestrator', 'Orchestrator', 'thread-1', '2026-01-01T00:00:00Z');",
    );
}

/// Counts written per legacy store, so the assertions read against the same
/// numbers the fixture was built from.
struct Fixture {
    approvals: usize,
    devices: usize,
    notifications: usize,
    task_sources: usize,
    mcp_servers: usize,
    mcp_audit: usize,
    connector_prefs: usize,
    trigger_days: [(&'static str, usize); 2],
    x402_payments: usize,
    ccr_originals: usize,
}

/// A whole pre-storage desktop workspace. Returns what was put in it.
fn desktop_workspace(workspace: &Path) -> Fixture {
    legacy_sessions(workspace);
    // Flows, cron and checkpoints: the committed fixture, byte for byte.
    copy_tree(&tinyflows_fixture(), workspace);

    // Approvals: two requests (one undecided) and a flow trust grant.
    sqlite_file(
        workspace,
        "approval/approval.db",
        "CREATE TABLE pending_approvals (
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
         VALUES ('still-pending', 'shell', 'run ls', '{\"cmd\":\"ls\"}', 'sess',
                 '2026-01-01T00:00:00+00:00', '2099-01-01T00:00:00+00:00');
         INSERT INTO pending_approvals
             (request_id, tool_name, action_summary, args_redacted, session_id, created_at,
              expires_at, decided_at, decision)
         VALUES ('already-approved', 'shell', 'run pwd', '{}', 'sess',
                 '2026-01-01T00:00:00+00:00', '2099-01-01T00:00:00+00:00',
                 '2026-01-01T00:01:00+00:00', 'approve_once');
         INSERT INTO flow_tool_trust (flow_id, tool_name, created_at)
         VALUES ('flow-1', 'shell', '2026-01-01T00:00:00+00:00');",
    );

    // Paired devices.
    sqlite_file(
        workspace,
        "devices/devices.db",
        "CREATE TABLE paired_devices (
             channel_id TEXT PRIMARY KEY, label TEXT NOT NULL, device_pubkey TEXT NOT NULL,
             core_session_token_hash TEXT NOT NULL, shared_secret_encrypted BLOB,
             created_at TEXT NOT NULL, last_seen_at TEXT, revoked INTEGER NOT NULL DEFAULT 0);
         INSERT INTO paired_devices
             (channel_id, label, device_pubkey, core_session_token_hash, created_at, revoked)
         VALUES ('chan-a', 'Phone', 'pk', 'hash', '2026-01-01T00:00:00+00:00', 0),
                ('chan-b', 'Old laptop', 'pk', 'hash', '2026-01-01T00:00:00+00:00', 1);",
    );

    // Notifications: an integration notification, its provider settings and a
    // core notification that fired while the app was closed.
    sqlite_file(
        workspace,
        "notifications/notifications.db",
        "CREATE TABLE integration_notifications (
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
             ('core-1', '{\"id\":\"core-1\",\"title\":\"cron done\"}', 1767225600000, 0,
              '2026-01-01T00:00:00+00:00');",
    );

    // Task sources and what they already ingested.
    sqlite_file(
        workspace,
        "task_sources/sources.db",
        "CREATE TABLE task_sources (
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
         VALUES ('src-1', 'github', '{\"provider\":\"github\"}', 600, '\"todo_only\"', 5,
                 '2026-01-01T00:00:00+00:00');
         PRAGMA user_version = 0;
         INSERT INTO ingested_tasks (source_id, external_id, content_hash, ingested_at)
         VALUES ('src-1', 'issue-9', 'abc', '2026-01-01T00:00:00+00:00');",
    );

    // tinymcp: servers (one with env secrets) and the write audit.
    sqlite_file(
        workspace,
        "mcp_clients/mcp_clients.db",
        "CREATE TABLE mcp_servers (
             server_id TEXT PRIMARY KEY, qualified_name TEXT NOT NULL, display_name TEXT NOT NULL,
             description TEXT, icon_url TEXT, command_kind TEXT NOT NULL DEFAULT 'node',
             command TEXT NOT NULL, args_json TEXT NOT NULL DEFAULT '[]',
             env_keys_json TEXT NOT NULL DEFAULT '[]', config_json TEXT,
             installed_at INTEGER NOT NULL, last_connected_at INTEGER);
         CREATE TABLE mcp_client_env (
             server_id TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
             PRIMARY KEY (server_id, key));
         INSERT INTO mcp_servers VALUES
             ('srv-1', 'io.example/files', 'Files', NULL, NULL, 'npx', 'files-mcp',
              '[]', '[\"API_KEY\"]', NULL, 1700000000000, NULL),
             ('srv-2', 'io.example/plain', 'Plain', NULL, NULL, 'node', 'plain.js',
              '[]', '[]', NULL, 1700000100000, NULL);
         INSERT INTO mcp_client_env VALUES ('srv-1', 'API_KEY', 'sk-secret');",
    );
    sqlite_file(
        workspace,
        "mcp_audit/mcp_audit.db",
        "CREATE TABLE mcp_writes (
             id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp_ms INTEGER NOT NULL,
             client_info TEXT NOT NULL, tool_name TEXT NOT NULL, args_summary TEXT,
             resulting_chunk_id TEXT, success INTEGER NOT NULL, error_message TEXT);
         INSERT INTO mcp_writes (timestamp_ms, client_info, tool_name, success)
         VALUES (10, 'claude', 'memory.write', 1), (20, 'claude', 'memory.write', 0);",
    );

    // Connector preferences and the daily trigger archive.
    write(
        workspace,
        "composio-user-scopes/gmail.json",
        r#"{"allow_write":false,"allow_delete":false}"#,
    );
    write(workspace, "composio-user-scopes/slack.json", "true");
    write(
        workspace,
        "triggers/2026-10-12.jsonl",
        concat!(
            r#"{"receivedAtMs":1,"toolkit":"gmail","trigger":"NEW","metadataId":"a","metadataUuid":"u","payload":{}}"#,
            "\n",
            r#"{"receivedAtMs":2,"toolkit":"gmail","trigger":"NEW","metadataId":"b","metadataUuid":"v","payload":{}}"#,
            "\n"
        ),
    );
    write(
        workspace,
        "triggers/2026-10-13.jsonl",
        concat!(
            r#"{"receivedAtMs":3,"toolkit":"slack","trigger":"MSG","metadataId":"c","metadataUuid":"w","payload":{}}"#,
            "\n"
        ),
    );

    // x402 payment ledger: a payment that changed status appears twice.
    let payment = |id: &str, status: &str| {
        format!(
            r#"{{"id":"{id}","url":"https://api.example/x","asset":"0xusdc","amountAtomic":2500,"amountDisplay":"0.002500 USDC","recipient":"0xpayee","network":"eip155:8453","txSignature":null,"status":"{status}","timestamp":"2026-10-12T10:00:00Z"}}"#
        )
    };
    write(
        workspace,
        "x402/payments.jsonl",
        &format!(
            "{}\n{}\n{}\n",
            payment("p1", "pending"),
            payment("p2", "settled"),
            payment("p1", "settled")
        ),
    );

    // tinybox jail registry.
    write(
        workspace,
        "jails/index.json",
        r#"{"jails":{"j1":{"id":"j1","label":"build","dir":"/w/jails/j1","backend_at_create":"landlock","created_at_unix":10,"updated_at_unix":20,"notes":"keep"}}}"#,
    );

    // tinyjuice CCR tier: two originals under their tokens.
    let ccr = workspace.join("ccr");
    std::fs::create_dir_all(&ccr).unwrap();
    for (token, text) in [
        ("00112233445566778899aabbccddeeff", "first original"),
        ("ffeeddccbbaa99887766554433221100", "second original"),
    ] {
        std::fs::write(ccr.join(token), text).unwrap();
        let file = std::fs::File::options()
            .write(true)
            .open(ccr.join(token))
            .unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(60))
            .unwrap();
    }

    Fixture {
        approvals: 3,
        devices: 2,
        notifications: 3,
        task_sources: 2,
        mcp_servers: 2,
        mcp_audit: 2,
        connector_prefs: 2,
        trigger_days: [("2026-10-12", 2), ("2026-10-13", 1)],
        x402_payments: 3,
        ccr_originals: 2,
    }
}

fn module_storage(backend: &dyn StorageBackend, module: &str) -> ScopedStorage {
    backend
        .for_scope(&Scope::local().module(module).unwrap())
        .unwrap()
}

async fn count(scoped: &ScopedStorage, collection: &str) -> usize {
    scoped
        .documents()
        .query_all(collection, &Query::all())
        .await
        .unwrap()
        .len()
}

/// Total documents the host's own sources keep in the local scope.
async fn host_documents(backend: &dyn StorageBackend) -> usize {
    let docs = backend.for_scope(&Scope::local()).unwrap();
    let mut total = 0;
    // The importers write the v2 collections of the owners behind each source.
    let owners: Vec<&str> = import::sources()
        .iter()
        .map(|source| source.owner)
        .collect();
    for schema in openhuman_core::storage::schema::owner_schemas() {
        if !owners.contains(&schema.owner.as_str()) {
            continue;
        }
        for item in &schema.items {
            if let tinystoragedrivers_core::schema::Item::Collection { spec, .. } = item {
                docs.documents().ensure_collection(spec).await.unwrap();
                total += count(&docs, &spec.name).await;
            }
        }
    }
    total
}

async fn import_modules(workspace: &Path) {
    use openhuman_core::modules;
    let ctx = openhuman_core::core::runtime::DispatchCtx::process_default();
    modules::import_legacy_mcp(&ctx, workspace).await;
    modules::import_legacy_connectors(&ctx, workspace).await;
    #[cfg(feature = "web3")]
    assert!(modules::import_legacy_x402(&ctx, workspace).await);
    modules::import_legacy_tinybox(&ctx, workspace).await;
    modules::import_legacy_tinyjuice(
        &ctx,
        &workspace.join("ccr"),
        Some(Duration::from_secs(3600)),
    )
    .await;
}

async fn a_desktop_upgrade_is_lossless_and_leaves_the_legacy_files_untouched(case: Case) {
    // TinyAgents' run ledger imports `sessions.db` (and appends to it) with
    // checked batches, which a driver without `ValidatedBatch` refuses.
    let local = case.backend().for_scope(&Scope::local()).unwrap();
    if !local
        .documents()
        .capabilities()
        .contains(Capability::ValidatedBatch)
    {
        // Written to the stderr handle, not with `eprintln!`, which the test
        // harness captures for a passing test: the skip must show in the log.
        use std::io::Write;
        let _ = writeln!(
            std::io::stderr(),
            "skipped {}: the driver lacks the validated_batch capability the \
             tinyagents run ledger needs to import sessions.db",
            case.driver.name()
        );
        return;
    }

    std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "file");
    std::env::set_var("OPENHUMAN_KEYRING_MASTER_KEY", "33".repeat(32));

    let workspace_dir = tempfile::tempdir().unwrap();
    let workspace = workspace_dir.path();
    std::env::set_var("OPENHUMAN_WORKSPACE", workspace);
    let fixture = desktop_workspace(workspace);
    let before = tree(workspace);
    assert!(
        before.len() > 15,
        "the fixture is a whole workspace: {before:#?}"
    );

    case.install();
    let backend = case.backend();
    let config = Config {
        workspace_dir: workspace.to_path_buf(),
        ..Config::default()
    };

    // First boot: every host importer runs and none is left without a source.
    let reports = import::run(
        &openhuman_core::core::runtime::DispatchCtx::process_default(),
        &config,
    )
    .await;
    assert!(!reports.is_empty());
    for report in &reports {
        match report.outcome.as_ref().unwrap_or_else(|error| {
            panic!("{} failed to import: {error:#}", report.domain);
        }) {
            Outcome::Imported { found, written } => {
                assert!(*found > 0, "{} found nothing", report.domain);
                assert_eq!(found, written, "{} lost records", report.domain);
            }
            other => panic!("{}: expected an import, got {other:?}", report.domain),
        }
    }
    let domains: Vec<_> = reports.iter().map(|r| r.domain).collect();
    for expected in ["approval", "devices", "notifications", "task_sources"] {
        assert!(domains.contains(&expected), "{expected} in {domains:?}");
    }
    assert_eq!(
        host_documents(backend.as_ref()).await,
        fixture.approvals + fixture.devices + fixture.notifications + fixture.task_sources
    );

    import_modules(workspace).await;

    // tinymcp: both servers (plus their name claims), the audit log and the
    // env secret, which is never a document.
    let mcp = module_storage(backend.as_ref(), "tinymcp");
    assert_eq!(count(&mcp, "mcp.servers").await, fixture.mcp_servers);
    assert_eq!(count(&mcp, "mcp.server_names").await, fixture.mcp_servers);
    assert_eq!(
        mcp.streams().len("mcp.audit").await.unwrap(),
        fixture.mcp_audit as u64
    );
    let stored = mcp
        .documents()
        .get("mcp.servers", "srv-1")
        .await
        .unwrap()
        .unwrap();
    assert!(!stored.doc.to_string().contains("sk-secret"));

    // Connectors: preferences and each day's archive in its own stream.
    let connectors = module_storage(backend.as_ref(), "tinyconnectors");
    assert_eq!(
        count(&connectors, "connectors.user_scopes").await,
        fixture.connector_prefs
    );
    for (day, records) in fixture.trigger_days {
        assert_eq!(
            connectors
                .streams()
                .len(&format!("connectors.triggers/{day}"))
                .await
                .unwrap(),
            records as u64,
            "trigger archive {day}"
        );
    }

    // x402 ledger (wallet gate).
    #[cfg(feature = "web3")]
    {
        let wallet = module_storage(backend.as_ref(), "tinywallet");
        assert_eq!(
            wallet.streams().len("wallet.payments").await.unwrap(),
            fixture.x402_payments as u64
        );
    }
    #[cfg(not(feature = "web3"))]
    let _ = fixture.x402_payments;

    // tinybox jail index.
    let tinybox = module_storage(backend.as_ref(), "tinybox");
    let index = tinybox
        .documents()
        .get("box.jail_index", "jails")
        .await
        .unwrap()
        .expect("the jail index was imported");
    assert_eq!(index.doc["jails"]["j1"]["label"], "build");

    // tinyjuice originals.
    let juice = module_storage(backend.as_ref(), "tinyjuice");
    for token in [
        "00112233445566778899aabbccddeeff",
        "ffeeddccbbaa99887766554433221100",
    ] {
        assert!(
            juice
                .blobs()
                .get(&format!("juice/ccr/{token}"))
                .await
                .unwrap()
                .is_some(),
            "CCR original {token}"
        );
    }
    let _ = fixture.ccr_originals;

    // Second boot: nothing runs again, so nothing deleted comes back.
    let again = import::run(
        &openhuman_core::core::runtime::DispatchCtx::process_default(),
        &config,
    )
    .await;
    assert_eq!(again.len(), reports.len());
    for report in &again {
        assert_eq!(
            report.outcome.as_ref().unwrap(),
            &Outcome::AlreadyDone,
            "{} imported twice",
            report.domain
        );
    }
    assert_eq!(
        host_documents(backend.as_ref()).await,
        fixture.approvals + fixture.devices + fixture.notifications + fixture.task_sources
    );

    // The user's files were only ever read.
    assert_eq!(tree(workspace), before, "the legacy files changed");
}

driver_cases!(async a_desktop_upgrade_is_lossless_and_leaves_the_legacy_files_untouched);
