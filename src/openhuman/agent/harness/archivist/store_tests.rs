//! Tests for the archivist's md-backed episodic capture store.
//!
//! Two halves, and the second one is the point.
//!
//! The first half is the behaviour suite the store came home with: round
//! trips, sequence assignment, concurrent writers, session isolation.
//!
//! The second half is the **migration proof**. This store was moved out of
//! `tinycortex` and back, and every turn any user has ever archived is already
//! on disk in the format the engine's copy wrote. "The port is faithful" is
//! therefore not a claim to assert, it is a claim to *check against the thing
//! being replaced* — so [`derived_paths_and_bytes_match_the_engine_store`]
//! writes the same turns through both implementations into two workspaces and
//! compares the resulting trees byte for byte, and the two cross-read tests
//! pin that each implementation reads the other's files.
//!
//! The engine stays a `dev-dependency` for exactly this kind of fixture, which
//! is why naming `tinycortex` here is fine while naming it in `hook_impl.rs`
//! is the thing being removed.

use super::*;
use std::collections::BTreeMap;
use tempfile::TempDir;

fn turn(session: &str, role: &str, content: &str) -> ArchivedTurn {
    ArchivedTurn {
        session_id: session.into(),
        seq: 0,
        timestamp_ms: 1_700_000_000_000,
        role: role.into(),
        content: content.into(),
        lesson: None,
        tool_calls_json: None,
        cost_microdollars: 0,
    }
}

#[test]
fn round_trip_single_turn() {
    let tmp = TempDir::new().unwrap();
    let stored = record_turn(tmp.path(), turn("s1", "user", "hello world")).unwrap();
    assert_eq!(stored.seq, 0);
    let read = session_entries(tmp.path(), "s1").unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].content, "hello world");
    assert_eq!(read[0].role, "user");
    assert_eq!(read[0].session_id, "s1");
    assert_eq!(read[0].seq, 0);
}

#[test]
fn append_increments_seq() {
    let tmp = TempDir::new().unwrap();
    let a = record_turn(tmp.path(), turn("s1", "user", "one")).unwrap();
    let b = record_turn(tmp.path(), turn("s1", "assistant", "two")).unwrap();
    let c = record_turn(tmp.path(), turn("s1", "user", "three")).unwrap();
    assert_eq!((a.seq, b.seq, c.seq), (0, 1, 2));
    let read = session_entries(tmp.path(), "s1").unwrap();
    assert_eq!(
        read.iter().map(|t| t.seq).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(read[1].role, "assistant");
    assert_eq!(read[2].content, "three");
}

#[test]
fn concurrent_record_turn_retries_sequence_collisions_without_loss() {
    let tmp = TempDir::new().unwrap();
    let writers = 24;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(writers));
    let mut threads = Vec::new();
    for index in 0..writers {
        let workspace = tmp.path().to_path_buf();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            record_turn(&workspace, turn("shared", "user", &format!("turn-{index}"))).unwrap()
        }));
    }
    let mut assigned = Vec::new();
    for thread in threads {
        assigned.push(thread.join().unwrap().seq);
    }
    assigned.sort_unstable();
    assert_eq!(assigned, (0..writers as u32).collect::<Vec<_>>());

    let entries = session_entries(tmp.path(), "shared").unwrap();
    assert_eq!(entries.len(), writers);
    let contents: std::collections::HashSet<_> =
        entries.into_iter().map(|entry| entry.content).collect();
    assert_eq!(contents.len(), writers);
}

#[test]
fn missing_session_returns_empty() {
    let tmp = TempDir::new().unwrap();
    assert!(session_entries(tmp.path(), "never").unwrap().is_empty());
}

#[test]
fn preserves_lesson_and_tool_calls() {
    let tmp = TempDir::new().unwrap();
    let mut t = turn("s1", "assistant", "did the thing");
    t.lesson = Some("be careful with X: it bites".into());
    t.tool_calls_json = Some(r#"[{"name":"bash","args":{"cmd":"ls"}}]"#.into());
    t.cost_microdollars = 1234;
    record_turn(tmp.path(), t.clone()).unwrap();
    let read = session_entries(tmp.path(), "s1").unwrap();
    assert_eq!(
        read[0].lesson.as_deref(),
        Some("be careful with X: it bites")
    );
    assert_eq!(
        read[0].tool_calls_json.as_deref(),
        Some(r#"[{"name":"bash","args":{"cmd":"ls"}}]"#)
    );
    assert_eq!(read[0].cost_microdollars, 1234);
}

#[test]
fn front_matter_round_trips_multiline_and_delimiter_like_scalars() {
    let tmp = TempDir::new().unwrap();
    let mut t = turn("session:one", "assistant\nadmin", "body stays separate");
    t.lesson = Some("first line\n---\nsecond: line\\tail".into());
    record_turn(tmp.path(), t.clone()).unwrap();

    let read = session_entries(tmp.path(), &t.session_id).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(read[0].session_id, t.session_id);
    assert_eq!(read[0].role, t.role);
    assert_eq!(read[0].lesson, t.lesson);
    assert_eq!(read[0].content, t.content);
}

#[test]
fn unsafe_session_ids_have_collision_resistant_directories() {
    let tmp = TempDir::new().unwrap();
    record_turn(tmp.path(), turn("a/b", "user", "slash")).unwrap();
    record_turn(tmp.path(), turn("a?b", "user", "question")).unwrap();

    assert_eq!(
        session_entries(tmp.path(), "a/b").unwrap()[0].content,
        "slash"
    );
    assert_eq!(
        session_entries(tmp.path(), "a?b").unwrap()[0].content,
        "question"
    );
}

#[test]
fn distinct_sessions_dont_mix() {
    let tmp = TempDir::new().unwrap();
    record_turn(tmp.path(), turn("a", "user", "hi a")).unwrap();
    record_turn(tmp.path(), turn("b", "user", "hi b")).unwrap();
    record_turn(tmp.path(), turn("a", "user", "more a")).unwrap();
    let a = session_entries(tmp.path(), "a").unwrap();
    let b = session_entries(tmp.path(), "b").unwrap();
    assert_eq!(a.len(), 2);
    assert_eq!(b.len(), 1);
    assert_eq!(b[0].content, "hi b");
}

/// The literal path an archived turn lands at, spelled out rather than
/// derived, so a change to `content_root` / the `{:06}` filename fails here
/// instead of silently orphaning every existing archive.
#[test]
fn derived_path_is_the_documented_one() {
    let tmp = TempDir::new().unwrap();
    record_turn(tmp.path(), turn("sess-1", "user", "hi")).unwrap();
    let expected = tmp
        .path()
        .join("memory_tree")
        .join("content")
        .join("episodic")
        .join("sess-1")
        .join("000000.md");
    assert!(expected.is_file(), "expected {}", expected.display());
}

// ── Engine-equivalence: the migration proof ──────────────────────────────────

/// The turns both implementations are driven with. Deliberately covers every
/// branch the on-disk bytes can take: the two optional front-matter keys
/// present and absent, a body with and without a trailing newline, a scalar
/// that has to be escaped, a session id that has to be sanitised, and a
/// non-zero cost.
fn equivalence_fixtures() -> Vec<ArchivedTurn> {
    let mut with_extras = turn("sess-a", "assistant", "did the thing");
    with_extras.lesson = Some("careful: it\nbites\n---\nstill".into());
    with_extras.tool_calls_json = Some(r#"[{"name":"bash","args":{"cmd":"ls"}}]"#.into());
    with_extras.cost_microdollars = 4_242;

    let mut trailing_newline = turn("sess-a", "user", "body already ends in a newline\n");
    trailing_newline.timestamp_ms = -1;

    vec![
        turn("sess-a", "user", "plain body"),
        with_extras,
        trailing_newline,
        turn("weird/id?x", "user", "sanitised session directory"),
        turn("", "system", "empty session id"),
        turn(
            "s\u{00e9}ance-\u{4e2d}\u{6587}",
            "user",
            "unicode session id",
        ),
    ]
}

/// Same shape as [`ArchivedTurn`], on the engine's type, so both stores are
/// driven with the identical payload.
fn to_engine(turn: &ArchivedTurn) -> tinycortex::memory::archivist::types::ArchivedTurn {
    tinycortex::memory::archivist::types::ArchivedTurn {
        session_id: turn.session_id.clone(),
        seq: turn.seq,
        timestamp_ms: turn.timestamp_ms,
        role: turn.role.clone(),
        content: turn.content.clone(),
        lesson: turn.lesson.clone(),
        tool_calls_json: turn.tool_calls_json.clone(),
        cost_microdollars: turn.cost_microdollars,
    }
}

/// Every file under `root`, keyed by its path relative to `root` (forward
/// slashes) and valued by its exact bytes.
fn tree_snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                let rel = path
                    .strip_prefix(root)
                    .expect("walked path is under root")
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                out.insert(rel, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// **The load-bearing test.** Drive both implementations with the same turns
/// into two workspaces and assert the resulting trees are identical — same
/// relative paths, same bytes. A one-character difference in the front matter,
/// the filename width, the sanitiser's digest, or the content root fails here.
#[test]
fn derived_paths_and_bytes_match_the_engine_store() {
    let host_ws = TempDir::new().unwrap();
    let engine_ws = TempDir::new().unwrap();
    let engine_cfg = tinycortex::memory::MemoryConfig::new(engine_ws.path().to_path_buf());

    for fixture in equivalence_fixtures() {
        let host_seq = record_turn(host_ws.path(), fixture.clone()).unwrap().seq;
        let engine_seq =
            tinycortex::memory::archivist::store::record_turn(&engine_cfg, to_engine(&fixture))
                .unwrap()
                .seq;
        assert_eq!(
            host_seq, engine_seq,
            "assigned seq diverged for session {:?}",
            fixture.session_id
        );
    }

    let host_tree = tree_snapshot(host_ws.path());
    let engine_tree = tree_snapshot(engine_ws.path());

    assert!(
        !host_tree.is_empty(),
        "snapshot is empty — the walker, not the store, is what this would be testing"
    );
    assert_eq!(
        host_tree.keys().collect::<Vec<_>>(),
        engine_tree.keys().collect::<Vec<_>>(),
        "derived on-disk paths diverged from the engine store"
    );
    for (path, host_bytes) in &host_tree {
        assert_eq!(
            String::from_utf8_lossy(host_bytes),
            String::from_utf8_lossy(&engine_tree[path]),
            "file contents diverged from the engine store at {path}"
        );
    }
}

/// Turns already on disk — written by the engine's copy before the move — must
/// read back through the store that replaced it. This is the upgrade path.
#[test]
fn reads_back_turns_the_engine_store_wrote() {
    let workspace = TempDir::new().unwrap();
    let engine_cfg = tinycortex::memory::MemoryConfig::new(workspace.path().to_path_buf());
    for fixture in equivalence_fixtures() {
        tinycortex::memory::archivist::store::record_turn(&engine_cfg, to_engine(&fixture))
            .unwrap();
    }

    for fixture in equivalence_fixtures() {
        let read = session_entries(workspace.path(), &fixture.session_id).unwrap();
        let found = read
            .iter()
            .find(|t| t.content == fixture.content.trim_end())
            .unwrap_or_else(|| panic!("no engine-written turn matched {:?}", fixture.content));
        assert_eq!(found.session_id, fixture.session_id);
        assert_eq!(found.role, fixture.role);
        assert_eq!(found.lesson, fixture.lesson);
        assert_eq!(found.tool_calls_json, fixture.tool_calls_json);
        assert_eq!(found.cost_microdollars, fixture.cost_microdollars);
        assert_eq!(found.timestamp_ms, fixture.timestamp_ms);
    }
}

/// And the other direction: a rollback, or a workspace shared with any engine
/// build still running the old code, must still read what this store wrote.
#[test]
fn the_engine_store_reads_back_turns_this_store_wrote() {
    let workspace = TempDir::new().unwrap();
    for fixture in equivalence_fixtures() {
        record_turn(workspace.path(), fixture.clone()).unwrap();
    }

    let engine_cfg = tinycortex::memory::MemoryConfig::new(workspace.path().to_path_buf());
    for fixture in equivalence_fixtures() {
        let read =
            tinycortex::memory::archivist::store::session_entries(&engine_cfg, &fixture.session_id)
                .unwrap();
        assert!(
            read.iter().any(|t| t.content == fixture.content.trim_end()
                && t.role == fixture.role
                && t.lesson == fixture.lesson
                && t.tool_calls_json == fixture.tool_calls_json
                && t.cost_microdollars == fixture.cost_microdollars),
            "the engine store could not read back {:?}",
            fixture.content
        );
    }
}
