//! Fixture-matrix tests for the session importer — one test per row of the
//! matrix in `docs/tinyagents-session-migration-design.md`.

use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use tinyagents::harness::store::{AppendStore, FileStore, JsonlAppendStore, Store};

use super::convert::sanitize_store_name;
use super::ops::{run_import, store_root};
use super::types::{
    ImportOptions, ImportSummary, ItemAction, JournalMessage, SessionDescriptor, MARKER_KEY,
    NS_MIGRATIONS, NS_SESSIONS,
};
use crate::openhuman::agent::harness::session::transcript::{
    read_transcript, read_transcript_legacy_md,
};

fn ws() -> TempDir {
    TempDir::new().expect("tempdir")
}

fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn flat_jsonl(ws: &Path, stem: &str) -> PathBuf {
    ws.join("session_raw").join(format!("{stem}.jsonl"))
}

fn meta_line(thread_id: Option<&str>, dispatcher: &str) -> String {
    let thread = thread_id
        .map(|t| format!(",\"thread_id\":\"{t}\""))
        .unwrap_or_default();
    format!(
        "{{\"_meta\":{{\"agent\":\"orchestrator\",\"agent_id\":\"orchestrator\",\
         \"dispatcher\":\"{dispatcher}\",\"provider\":\"anthropic\",\"model\":\"claude\",\
         \"created\":\"2024-01-01T00:00:00Z\",\"updated\":\"2024-01-01T00:05:00Z\",\
         \"turn_count\":1,\"input_tokens\":100,\"output_tokens\":50,\
         \"cached_input_tokens\":20,\"charged_amount_usd\":0.05{thread}}}}}\n"
    )
}

/// A native transcript: user turn + assistant turn carrying usage and a
/// tool call with Gemini-style `extra_content` passthrough.
fn native_body() -> &'static str {
    concat!(
        "{\"role\":\"user\",\"content\":\"hi\"}\n",
        "{\"role\":\"assistant\",\"content\":\"done\",\"provider\":\"anthropic\",",
        "\"model\":\"claude\",\"usage\":{\"input\":100,\"output\":50,\"cached_input\":20,",
        "\"context_window\":200000,\"cost_usd\":0.05},\"ts\":\"2024-01-01T00:00:01Z\",",
        "\"iteration\":1,\"tool_calls\":[{\"id\":\"tc1\",\"name\":\"read_file\",",
        "\"arguments\":\"{\\\"path\\\":\\\"x\\\"}\",",
        "\"extra_content\":{\"google\":{\"thought_signature\":\"sig\"}}}]}\n",
    )
}

async fn run(ws: &Path, opts: ImportOptions) -> ImportSummary {
    run_import(ws, &opts).await.expect("run_import")
}

async fn descriptor(ws: &Path, stem: &str) -> SessionDescriptor {
    let kv = FileStore::new(store_root(ws).join("kv"));
    let value = kv
        .get(NS_SESSIONS, &sanitize_store_name(stem))
        .await
        .expect("kv get")
        .unwrap_or_else(|| panic!("descriptor missing for {stem}"));
    serde_json::from_value(value).expect("descriptor shape")
}

async fn journal_readback(ws: &Path, stream: &str) -> Vec<JournalMessage> {
    let journal = JsonlAppendStore::new(store_root(ws).join("journal"));
    journal
        .read_from(stream, 0)
        .await
        .expect("journal read")
        .into_iter()
        .map(|(_, v)| serde_json::from_value(v).expect("journal record shape"))
        .collect()
}

/// Parity: the journal read-back must equal what `read_transcript` returns
/// for the source, field for field (including reattached turn-usage
/// metadata).
async fn assert_parity_jsonl(ws: &Path, stem: &str, source: &Path) {
    let expected: Vec<JournalMessage> = read_transcript(source)
        .expect("read_transcript")
        .messages
        .iter()
        .map(JournalMessage::from)
        .collect();
    let actual = journal_readback(ws, &format!("session.{stem}.messages")).await;
    assert_eq!(actual, expected, "journal read-back diverges for {stem}");
}

// Fixture 1 + 7: current flat layout, native dispatcher, tool_calls with
// extra_content — full parity + descriptor mapping.
#[tokio::test]
async fn imports_flat_native_jsonl_with_parity() {
    let ws = ws();
    let stem = "1719000000_orchestrator";
    let source = flat_jsonl(ws.path(), stem);
    write_file(
        &source,
        &(meta_line(Some("t-root"), "native") + native_body()),
    );

    let summary = run(ws.path(), ImportOptions::default()).await;
    assert_eq!(summary.scanned, 1);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.messages_written, 2);

    let desc = descriptor(ws.path(), stem).await;
    assert_eq!(desc.session_key, stem);
    assert_eq!(desc.thread_id, "t-root");
    assert!(!desc.thread_id_synthesized);
    assert_eq!(desc.parent_session_key, None);
    assert_eq!(desc.dispatcher, "native");
    assert_eq!(desc.usage.input, 100);
    assert_eq!(desc.usage.cost_usd, 0.05);
    assert_eq!(
        desc.source.jsonl.as_deref(),
        Some("session_raw/1719000000_orchestrator.jsonl")
    );

    assert_parity_jsonl(ws.path(), stem, &source).await;

    // Turn-usage metadata (incl. the tool call's extra_content) survived.
    let messages = journal_readback(ws.path(), &format!("session.{stem}.messages")).await;
    let assistant = &messages[1];
    let usage = assistant
        .extra_metadata
        .as_ref()
        .and_then(|m| m.get("openhuman_turn_usage"))
        .expect("turn usage metadata");
    assert_eq!(usage["tool_calls"][0]["id"], "tc1");
    assert_eq!(
        usage["tool_calls"][0]["extra_content"]["google"]["thought_signature"],
        "sig"
    );

    // Global marker written after a full non-dry run.
    let kv = FileStore::new(store_root(ws.path()).join("kv"));
    assert!(kv.get(NS_MIGRATIONS, MARKER_KEY).await.unwrap().is_some());
}

// Fixture 2: legacy date-folder layout.
#[tokio::test]
async fn imports_legacy_date_folder_jsonl() {
    let ws = ws();
    let stem = "1718000000_researcher";
    let source = ws
        .path()
        .join("session_raw")
        .join("01062024")
        .join(format!("{stem}.jsonl"));
    write_file(
        &source,
        &(meta_line(Some("t-legacy"), "native") + native_body()),
    );

    let summary = run(ws.path(), ImportOptions::default()).await;
    assert_eq!(summary.imported, 1, "warnings: {:?}", summary.warnings);
    let desc = descriptor(ws.path(), stem).await;
    assert_eq!(
        desc.source.jsonl.as_deref(),
        Some("session_raw/01062024/1718000000_researcher.jsonl")
    );
    assert_parity_jsonl(ws.path(), stem, &source).await;
}

// Fixture 3: Markdown-only session via the legacy `<!--MSG-->` reader.
#[tokio::test]
async fn imports_markdown_only_session() {
    let ws = ws();
    let stem = "1717000000_helper";
    let md = ws
        .path()
        .join("sessions")
        .join("2024_06_01")
        .join(format!("{stem}.md"));
    write_file(
        &md,
        "<!-- session_transcript\nagent: helper\ndispatcher: native\n\
         created: 2024-06-01T00:00:00Z\nupdated: 2024-06-01T00:05:00Z\n\
         turn_count: 1\ninput_tokens: 10\noutput_tokens: 5\ncached_input_tokens: 0\n\
         charged_usd: $0.01\nthread_id: t-md\n-->\n\
         <!--MSG role=\"user\"-->\nhello\n<!--/MSG-->\n\
         <!--MSG role=\"assistant\"-->\nhi there\n<!--/MSG-->\n",
    );

    let summary = run(ws.path(), ImportOptions::default()).await;
    assert_eq!(summary.imported, 1, "warnings: {:?}", summary.warnings);

    let desc = descriptor(ws.path(), stem).await;
    assert_eq!(desc.thread_id, "t-md");
    assert_eq!(desc.source.jsonl, None);
    assert!(desc.source.md.as_deref().unwrap().ends_with(".md"));

    let expected: Vec<JournalMessage> = read_transcript_legacy_md(&md)
        .unwrap()
        .messages
        .iter()
        .map(JournalMessage::from)
        .collect();
    let actual = journal_readback(ws.path(), &format!("session.{stem}.messages")).await;
    assert_eq!(actual, expected);
}

// Fixture 4: sub-agent stems, two-level chain → parent keys from the stem.
#[tokio::test]
async fn subagent_stem_chain_sets_parent_lineage() {
    let ws = ws();
    let root = "100_a";
    let child = "100_a__200_b";
    let grandchild = "100_a__200_b__300_c";
    for (stem, thread) in [(root, "t-a"), (child, "t-b"), (grandchild, "t-c")] {
        write_file(
            &flat_jsonl(ws.path(), stem),
            &(meta_line(Some(thread), "native") + native_body()),
        );
    }

    let summary = run(ws.path(), ImportOptions::default()).await;
    assert_eq!(summary.imported, 3);
    assert_eq!(descriptor(ws.path(), root).await.parent_session_key, None);
    assert_eq!(
        descriptor(ws.path(), child)
            .await
            .parent_session_key
            .as_deref(),
        Some("100_a")
    );
    assert_eq!(
        descriptor(ws.path(), grandchild)
            .await
            .parent_session_key
            .as_deref(),
        Some("100_a__200_b")
    );
}

// Fixtures 5 + 6: XML and P-format tool markup stays verbatim in content.
#[tokio::test]
async fn xml_and_pformat_markup_preserved_verbatim() {
    let ws = ws();
    let xml_stem = "1716000000_xmlagent";
    let xml_markup =
        "calling now <tool_call>{\"name\":\"shell\",\"arguments\":{\"cmd\":\"ls\"}}</tool_call>";
    write_file(
        &flat_jsonl(ws.path(), xml_stem),
        &format!(
            "{}{{\"role\":\"assistant\",\"content\":\"{}\"}}\n",
            meta_line(Some("t-xml"), "xml"),
            xml_markup.replace('"', "\\\"")
        ),
    );
    let p_stem = "1716000001_pfagent";
    let p_markup = "<tool_call>read_file[notes.txt|10]</tool_call>";
    write_file(
        &flat_jsonl(ws.path(), p_stem),
        &format!(
            "{}{{\"role\":\"assistant\",\"content\":\"{}\"}}\n",
            meta_line(Some("t-pf"), "pformat"),
            p_markup.replace('"', "\\\"")
        ),
    );

    let summary = run(ws.path(), ImportOptions::default()).await;
    assert_eq!(summary.imported, 2);

    let xml_msgs = journal_readback(ws.path(), &format!("session.{xml_stem}.messages")).await;
    assert_eq!(xml_msgs[0].content, xml_markup);
    assert_eq!(descriptor(ws.path(), xml_stem).await.dispatcher, "xml");

    let p_msgs = journal_readback(ws.path(), &format!("session.{p_stem}.messages")).await;
    assert_eq!(p_msgs[0].content, p_markup);
    assert_eq!(descriptor(ws.path(), p_stem).await.dispatcher, "pformat");
}

// Fixture 8: malformed sources fail per-item without aborting the batch;
// a truncated trailing line is tolerated by the reader (skip + warning).
#[tokio::test]
async fn malformed_sources_fail_without_aborting_batch() {
    let ws = ws();
    write_file(
        &flat_jsonl(ws.path(), "1715000000_good"),
        &(meta_line(Some("t-good"), "native") + native_body()),
    );
    // Missing `_meta` first line.
    write_file(
        &flat_jsonl(ws.path(), "1715000001_nometa"),
        "{\"role\":\"user\",\"content\":\"hi\"}\n",
    );
    // Empty file.
    write_file(&flat_jsonl(ws.path(), "1715000002_empty"), "");
    // Truncated last line after one valid message.
    write_file(
        &flat_jsonl(ws.path(), "1715000003_truncated"),
        &format!(
            "{}{{\"role\":\"user\",\"content\":\"ok\"}}\n{{\"role\":\"assistant\",\"conte",
            meta_line(Some("t-trunc"), "native")
        ),
    );

    let summary = run(ws.path(), ImportOptions::default()).await;
    assert_eq!(summary.scanned, 4);
    assert_eq!(summary.imported, 2, "good + truncated-but-tolerated");
    assert_eq!(summary.failed, 2, "no-meta + empty fail per-item");

    let truncated = journal_readback(ws.path(), "session.1715000003_truncated.messages").await;
    assert_eq!(truncated.len(), 1, "malformed trailing line skipped");

    // The batch still completed and the marker landed.
    let kv = FileStore::new(store_root(ws.path()).join("kv"));
    assert!(kv.get(NS_MIGRATIONS, MARKER_KEY).await.unwrap().is_some());
}

// Fixture 9: `_meta` without thread_id → synthesized stable id + warning.
#[tokio::test]
async fn synthesizes_thread_id_when_meta_lacks_one() {
    let ws = ws();
    let stem = "1714000000_nothread";
    write_file(
        &flat_jsonl(ws.path(), stem),
        &(meta_line(None, "native") + native_body()),
    );

    let summary = run(ws.path(), ImportOptions::default()).await;
    let desc = descriptor(ws.path(), stem).await;
    assert_eq!(desc.thread_id, format!("imported-{stem}"));
    assert!(desc.thread_id_synthesized);
    let item = &summary.items[0];
    assert!(
        item.warnings.iter().any(|w| w.contains("synthesized")),
        "warnings: {:?}",
        item.warnings
    );
}

// Fixture 10: run-ledger rows join run_ids; a disagreeing ledger parent
// thread is a warning, and the stem chain wins.
#[tokio::test]
async fn run_ledger_links_join_and_disagreement_warns() {
    let ws = ws();
    let root = "100_root";
    let child = "100_root__200_child";
    write_file(
        &flat_jsonl(ws.path(), root),
        &(meta_line(Some("t-root"), "native") + native_body()),
    );
    write_file(
        &flat_jsonl(ws.path(), child),
        &(meta_line(Some("t-child"), "native") + native_body()),
    );

    let db_dir = ws.path().join("session_db");
    fs::create_dir_all(&db_dir).unwrap();
    let conn = rusqlite::Connection::open(db_dir.join("sessions.db")).unwrap();
    conn.execute_batch(
        "CREATE TABLE agent_runs (id TEXT PRIMARY KEY, parent_thread_id TEXT, worker_thread_id TEXT);
         INSERT INTO agent_runs VALUES ('run-1', 't-DIFFERENT', 't-child');",
    )
    .unwrap();
    drop(conn);

    let summary = run(ws.path(), ImportOptions::default()).await;
    assert_eq!(summary.imported, 2, "warnings: {:?}", summary.warnings);

    let child_desc = descriptor(ws.path(), child).await;
    assert_eq!(child_desc.run_ids, vec!["run-1".to_string()]);
    assert_eq!(
        child_desc.parent_session_key.as_deref(),
        Some("100_root"),
        "stem chain wins"
    );
    let child_item = summary
        .items
        .iter()
        .find(|i| i.session_key == child)
        .unwrap();
    assert!(
        child_item.warnings.iter().any(|w| w.contains("disagrees")),
        "warnings: {:?}",
        child_item.warnings
    );
}

// Fixture 11: full rerun short-circuits on the marker; a targeted rerun
// skips unchanged items and re-imports only the touched one.
#[tokio::test]
async fn idempotent_rerun_skips_then_touch_reimports() {
    let ws = ws();
    let a = "1713000000_a";
    let b = "1713000001_b";
    for stem in [a, b] {
        write_file(
            &flat_jsonl(ws.path(), stem),
            &(meta_line(Some(&format!("t-{stem}")), "native") + native_body()),
        );
    }

    let first = run(ws.path(), ImportOptions::default()).await;
    assert_eq!(first.imported, 2);

    // Full rerun: global marker short-circuits.
    let second = run(ws.path(), ImportOptions::default()).await;
    assert!(second.already_done);
    assert_eq!(second.scanned, 0);

    // Targeted rerun: everything unchanged → skipped.
    let targeted = run(
        ws.path(),
        ImportOptions {
            only: Some("*".into()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(targeted.skipped, 2);
    assert_eq!(targeted.imported, 0);

    // Touch one source (size changes) → only it re-imports.
    let path = flat_jsonl(ws.path(), a);
    let mut contents = fs::read_to_string(&path).unwrap();
    contents.push_str("{\"role\":\"user\",\"content\":\"follow-up\"}\n");
    fs::write(&path, contents).unwrap();

    let after_touch = run(
        ws.path(),
        ImportOptions {
            only: Some("*".into()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(after_touch.imported, 1);
    assert_eq!(after_touch.skipped, 1);

    // The re-imported stream was reset, not appended twice.
    let msgs = journal_readback(ws.path(), &format!("session.{a}.messages")).await;
    assert_eq!(msgs.len(), 3);
}

// Dry-run: reports the plan, writes nothing.
#[tokio::test]
async fn dry_run_writes_nothing() {
    let ws = ws();
    write_file(
        &flat_jsonl(ws.path(), "1712000000_plan"),
        &(meta_line(Some("t-plan"), "native") + native_body()),
    );

    let summary = run(
        ws.path(),
        ImportOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .await;
    assert!(summary.dry_run);
    assert_eq!(summary.imported, 1);
    assert_eq!(summary.messages_written, 0);
    assert_eq!(summary.items[0].action, ItemAction::WouldImport);
    assert!(
        !store_root(ws.path()).join("kv").exists()
            && !store_root(ws.path()).join("journal").exists(),
        "dry run must not create store dirs"
    );
}

// Sources are never mutated: byte-identical after import.
#[tokio::test]
async fn sources_untouched_after_import() {
    let ws = ws();
    let source = flat_jsonl(ws.path(), "1711000000_untouched");
    let contents = meta_line(Some("t-u"), "native") + native_body();
    write_file(&source, &contents);

    run(ws.path(), ImportOptions::default()).await;
    assert_eq!(fs::read_to_string(&source).unwrap(), contents);
}
