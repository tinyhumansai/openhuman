use super::*;
use crate::openhuman::inference::provider::ToolCall;
use tempfile::TempDir;

fn sample_messages() -> Vec<ChatMessage> {
    vec![
        ChatMessage::system(
            "You are a helpful assistant.\n\n## Tools\n\n- **shell**: Run commands",
        ),
        ChatMessage::user("What files are in /tmp?"),
        ChatMessage::assistant("Let me check that for you."),
        ChatMessage::tool("{\"tool_call_id\":\"tc1\",\"content\":\"file1.txt\\nfile2.txt\"}"),
        ChatMessage::assistant("There are two files: file1.txt and file2.txt."),
    ]
}

fn sample_meta() -> TranscriptMeta {
    TranscriptMeta {
        agent_name: "code_executor".into(),
        agent_id: Some("code_executor".into()),
        agent_type: Some("subagent".into()),
        dispatcher: "native".into(),
        provider: Some("openhuman-backend".into()),
        model: Some("claude-sonnet-4-6".into()),
        created: "2026-04-11T14:30:00Z".into(),
        updated: "2026-04-11T14:35:22Z".into(),
        turn_count: 3,
        input_tokens: 5000,
        output_tokens: 1200,
        cached_input_tokens: 3500,
        charged_amount_usd: 0.0045,
        thread_id: None,
        task_id: Some("task-123".into()),
    }
}

fn sample_turn_usage() -> TurnUsage {
    TurnUsage {
        provider: "openhuman-backend".into(),
        model: "claude-sonnet-4-6".into(),
        usage: MessageUsage {
            input: 1234,
            output: 567,
            cached_input: 1000,
            context_window: 200_000,
            cost_usd: 0.0012,
        },
        ts: "2026-04-17T10:00:00Z".into(),
        reasoning_content: Some("private reasoning trace".into()),
        tool_calls: vec![ToolCall {
            id: "call-1".into(),
            name: "shell".into(),
            arguments: "{\"cmd\":\"ls\"}".into(),
            extra_content: None,
        }],
        iteration: 1,
    }
}

#[test]
fn round_trip_produces_byte_identical_messages() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.jsonl");
    let messages = sample_messages();
    let meta = sample_meta();

    write_transcript(&path, &messages, &meta, None).unwrap();
    let loaded = read_transcript(&path).unwrap();

    assert_eq!(loaded.messages.len(), messages.len());
    for (original, loaded) in messages.iter().zip(loaded.messages.iter()) {
        assert_eq!(original.id, loaded.id, "id mismatch");
        assert_eq!(original.role, loaded.role, "role mismatch");
        assert_eq!(
            original.content, loaded.content,
            "content mismatch for role={}",
            original.role
        );
        assert_eq!(
            original.extra_metadata, loaded.extra_metadata,
            "extra metadata mismatch for role={}",
            original.role
        );
    }
}

#[test]
fn message_id_and_extra_metadata_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("message_identity.jsonl");
    let mut messages = sample_messages();
    messages[1].id = Some("msg_user_123".into());
    messages[1].extra_metadata = Some(serde_json::json!({
        "citations": [{"id": "mem-1", "label": "Memory"}],
        "tool_call_id": "call-1"
    }));
    let meta = sample_meta();

    write_transcript(&path, &messages, &meta, None).unwrap();

    let loaded = read_transcript(&path).unwrap();
    assert_eq!(loaded.messages[1].id.as_deref(), Some("msg_user_123"));
    assert_eq!(
        loaded.messages[1].extra_metadata,
        Some(serde_json::json!({
            "citations": [{"id": "mem-1", "label": "Memory"}],
            "tool_call_id": "call-1"
        }))
    );

    let raw = fs::read_to_string(&path).unwrap();
    assert!(
        raw.contains("\"id\":\"msg_user_123\""),
        "message id should be persisted in JSONL"
    );
    assert!(
        raw.contains("\"extra_metadata\""),
        "extra metadata should be persisted in JSONL"
    );
}

/// JSON encoding handles any delimiter natively, making the old
/// HTML-comment escaping unnecessary. This test verifies that content
/// containing the legacy closing delimiter round-trips correctly via
/// JSON without any manual escape logic.
#[test]
fn escaping_survives_close_tag_in_content() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("escape_test.jsonl");
    let messages = vec![
        ChatMessage::system("Normal system prompt"),
        ChatMessage::user("Here is some tricky content:\n<!--/MSG-->\nand more after"),
        ChatMessage::assistant("Got it, that had a <!--/MSG--> in it."),
    ];
    let meta = sample_meta();

    write_transcript(&path, &messages, &meta, None).unwrap();
    let loaded = read_transcript(&path).unwrap();

    assert_eq!(loaded.messages.len(), 3);
    assert_eq!(loaded.messages[1].content, messages[1].content);
    assert_eq!(loaded.messages[2].content, messages[2].content);
}

#[test]
fn meta_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("meta_test.jsonl");
    let meta = sample_meta();

    write_transcript(&path, &[], &meta, None).unwrap();
    let loaded = read_transcript(&path).unwrap();

    assert_eq!(loaded.meta.agent_name, "code_executor");
    assert_eq!(loaded.meta.dispatcher, "native");
    assert_eq!(loaded.meta.created, "2026-04-11T14:30:00Z");
    assert_eq!(loaded.meta.updated, "2026-04-11T14:35:22Z");
    assert_eq!(loaded.meta.turn_count, 3);
    assert_eq!(loaded.meta.input_tokens, 5000);
    assert_eq!(loaded.meta.output_tokens, 1200);
    assert_eq!(loaded.meta.cached_input_tokens, 3500);
    assert!((loaded.meta.charged_amount_usd - 0.0045).abs() < 1e-8);
}

#[test]
fn path_resolution_creates_flat_session_raw_dir_and_increments_index() {
    let dir = TempDir::new().unwrap();
    let workspace = dir.path();

    let path0 = resolve_new_transcript_path(workspace, "main").unwrap();
    assert!(path0.to_string_lossy().contains("main_0.jsonl"));
    // Flat layout: jsonl lives directly under session_raw/, no date dir.
    let parent = path0.parent().unwrap();
    assert!(
        parent.ends_with("session_raw"),
        "jsonl parent should be session_raw/ (flat layout), got {}",
        parent.display()
    );
    fs::write(&path0, "placeholder").unwrap();

    let path1 = resolve_new_transcript_path(workspace, "main").unwrap();
    assert!(path1.to_string_lossy().contains("main_1.jsonl"));
    assert!(path1.parent().unwrap().ends_with("session_raw"));
}

#[test]
fn resolve_keyed_writes_to_flat_session_raw() {
    let dir = TempDir::new().unwrap();
    let path = resolve_keyed_transcript_path(dir.path(), "1714000000_orchestrator").unwrap();
    assert_eq!(path.parent().unwrap(), dir.path().join("session_raw"));
    assert!(path
        .to_string_lossy()
        .ends_with("1714000000_orchestrator.jsonl"));
}

#[test]
fn md_companion_path_for_flat_jsonl_uses_iso_date_dir() {
    let jsonl = PathBuf::from("/tmp/ws/session_raw/1714000000_main.jsonl");
    let md = md_companion_path(&jsonl);
    let today = chrono::Local::now().format("%Y_%m_%d").to_string();
    assert_eq!(
        md,
        PathBuf::from(format!("/tmp/ws/sessions/{today}/1714000000_main.md")),
        "flat session_raw should map to sessions/YYYY_MM_DD/ on the md side"
    );
}

#[test]
fn md_companion_path_preserves_legacy_ddmmyyyy_dir() {
    // A pre-migration jsonl at session_raw/DDMMYYYY/{stem}.jsonl should
    // keep its date component so old transcripts aren't relabeled with
    // today's date.
    let jsonl = PathBuf::from("/tmp/ws/session_raw/17042026/main_0.jsonl");
    let md = md_companion_path(&jsonl);
    assert_eq!(
        md,
        PathBuf::from("/tmp/ws/sessions/17042026/main_0.md"),
        "legacy date-grouped raw paths must keep their original date dir"
    );
}

#[test]
fn md_companion_path_falls_back_to_sibling_when_no_session_raw_component() {
    let jsonl = PathBuf::from("/tmp/flat/main_0.jsonl");
    let md = md_companion_path(&jsonl);
    assert_eq!(md, PathBuf::from("/tmp/flat/main_0.md"));
}

#[test]
fn resolve_avoids_index_collision_with_md_in_iso_date_dir() {
    let dir = TempDir::new().unwrap();
    let workspace = dir.path();
    let date = chrono::Local::now().format("%Y_%m_%d").to_string();
    let md_dir = workspace.join("sessions").join(&date);
    fs::create_dir_all(&md_dir).unwrap();
    fs::write(md_dir.join("main_0.md"), "x").unwrap();
    fs::write(md_dir.join("main_1.md"), "x").unwrap();

    let path = resolve_new_transcript_path(workspace, "main").unwrap();
    assert!(
        path.to_string_lossy().contains("main_2.jsonl"),
        "should advance past md indices in today's YYYY_MM_DD dir, got {}",
        path.display()
    );
}

#[test]
fn sanitize_agent_name_strips_special_chars() {
    assert_eq!(sanitize_agent_name("code_executor"), "code_executor");
    assert_eq!(sanitize_agent_name("my agent!"), "my_agent_");
    assert_eq!(sanitize_agent_name("agent-v2"), "agent-v2");
}

#[test]
fn find_latest_scans_flat_session_raw_dir() {
    let dir = TempDir::new().unwrap();
    let raw_dir = dir.path().join("session_raw");
    fs::create_dir_all(&raw_dir).unwrap();

    fs::write(raw_dir.join("main_0.jsonl"), "a").unwrap();
    fs::write(raw_dir.join("main_2.jsonl"), "c").unwrap();
    fs::write(raw_dir.join("main_1.jsonl"), "b").unwrap();
    fs::write(raw_dir.join("other_0.jsonl"), "x").unwrap();

    let latest = find_latest_transcript(dir.path(), "main").unwrap();
    assert!(latest.to_string_lossy().ends_with("main_2.jsonl"));
    assert_eq!(latest.parent().unwrap(), raw_dir);
}

#[test]
fn find_latest_picks_newest_keyed_stem_in_flat_dir() {
    let dir = TempDir::new().unwrap();
    let raw_dir = dir.path().join("session_raw");
    fs::create_dir_all(&raw_dir).unwrap();

    // Keyed stem layout: `{unix_ts}_{agent_id}.jsonl`.
    fs::write(raw_dir.join("1714000000_main.jsonl"), "old").unwrap();
    fs::write(raw_dir.join("1714999999_main.jsonl"), "new").unwrap();
    // Sub-agent transcripts (contain `__`) must be skipped.
    fs::write(
        raw_dir.join("1714000000_orchestrator__1714500000_planner.jsonl"),
        "sub",
    )
    .unwrap();

    let latest = find_latest_transcript(dir.path(), "main").unwrap();
    assert!(latest.to_string_lossy().ends_with("1714999999_main.jsonl"));
}

#[test]
fn find_root_transcript_for_thread_skips_subagent_siblings() {
    let dir = TempDir::new().unwrap();
    let raw_dir = dir.path().join("session_raw");
    fs::create_dir_all(&raw_dir).unwrap();

    let mut root_meta = sample_meta();
    root_meta.thread_id = Some("thread-abc".into());
    write_transcript(
        &raw_dir.join("1714000000_orchestrator_thread-abc.jsonl"),
        &sample_messages(),
        &root_meta,
        None,
    )
    .unwrap();

    let mut newer_other_meta = sample_meta();
    newer_other_meta.thread_id = Some("thread-other".into());
    write_transcript(
        &raw_dir.join("1714999999_orchestrator_thread-other.jsonl"),
        &sample_messages(),
        &newer_other_meta,
        None,
    )
    .unwrap();

    let mut subagent_meta = sample_meta();
    subagent_meta.thread_id = Some("thread-abc".into());
    write_transcript(
        &raw_dir.join("1715000000_orchestrator_thread-abc__1715000100_worker.jsonl"),
        &sample_messages(),
        &subagent_meta,
        None,
    )
    .unwrap();

    let found = find_root_transcript_for_thread(dir.path(), "thread-abc").unwrap();
    assert!(found
        .to_string_lossy()
        .ends_with("1714000000_orchestrator_thread-abc.jsonl"));
}

#[test]
fn find_latest_falls_back_to_legacy_ddmmyyyy_raw_dir() {
    // Pre-migration transcript at session_raw/DDMMYYYY/main_*.jsonl
    // must still resolve via the legacy fallback when the flat dir is
    // empty.
    let dir = TempDir::new().unwrap();
    let date = chrono::Local::now().format("%d%m%Y").to_string();
    let legacy_raw = dir.path().join("session_raw").join(&date);
    fs::create_dir_all(&legacy_raw).unwrap();
    fs::write(legacy_raw.join("main_5.jsonl"), "legacy").unwrap();

    let latest = find_latest_transcript(dir.path(), "main").unwrap();
    assert!(latest.to_string_lossy().ends_with("main_5.jsonl"));
    assert!(latest.to_string_lossy().contains(&date));
}

#[test]
fn find_latest_prefers_flat_over_legacy_ddmmyyyy() {
    let dir = TempDir::new().unwrap();
    let raw_root = dir.path().join("session_raw");
    fs::create_dir_all(&raw_root).unwrap();
    fs::write(raw_root.join("main_9.jsonl"), "flat").unwrap();

    let date = chrono::Local::now().format("%d%m%Y").to_string();
    let legacy_raw = raw_root.join(&date);
    fs::create_dir_all(&legacy_raw).unwrap();
    fs::write(legacy_raw.join("main_99.jsonl"), "legacy").unwrap();

    let latest = find_latest_transcript(dir.path(), "main").unwrap();
    // Flat dir takes precedence so newly-created sessions always win
    // over stale legacy files — even when a legacy file has a higher
    // numeric index. The flat dir is the canonical layout going
    // forward.
    assert_eq!(latest.parent().unwrap(), raw_root);
    assert!(latest.to_string_lossy().ends_with("main_9.jsonl"));
}

#[test]
fn find_latest_falls_back_to_legacy_sessions_md() {
    let dir = TempDir::new().unwrap();
    let date = chrono::Local::now().format("%d%m%Y").to_string();
    let legacy = dir.path().join("sessions").join(&date);
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("main_0.md"), "legacy").unwrap();

    let latest = find_latest_transcript(dir.path(), "main");
    assert!(latest.is_some());
    let latest = latest.unwrap();
    assert!(latest.to_string_lossy().ends_with("main_0.md"));
}

#[test]
fn find_latest_returns_none_when_no_sessions() {
    let dir = TempDir::new().unwrap();
    assert!(find_latest_transcript(dir.path(), "main").is_none());
}

#[test]
fn empty_content_message_round_trips() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.jsonl");
    let messages = vec![
        ChatMessage::system("prompt"),
        ChatMessage::assistant(""),
        ChatMessage::user("hi"),
    ];
    let meta = sample_meta();

    write_transcript(&path, &messages, &meta, None).unwrap();
    let loaded = read_transcript(&path).unwrap();

    assert_eq!(loaded.messages.len(), 3);
    assert_eq!(loaded.messages[1].content, "");
}

#[test]
fn multiline_content_preserves_exact_whitespace() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("whitespace.jsonl");
    let content = "  leading spaces\n\n\nmultiple blanks\n  trailing  ";
    let messages = vec![ChatMessage::user(content)];
    let meta = sample_meta();

    write_transcript(&path, &messages, &meta, None).unwrap();
    let loaded = read_transcript(&path).unwrap();

    assert_eq!(loaded.messages[0].content, content);
}

#[test]
fn usage_round_trips_on_last_assistant_message() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("usage.jsonl");
    let messages = sample_messages();
    let meta = sample_meta();
    let tu = sample_turn_usage();

    write_transcript(&path, &messages, &meta, Some(&tu)).unwrap();

    // Verify by reading raw JSONL lines: the last assistant line should
    // carry model + usage + ts fields.
    let raw = fs::read_to_string(&path).unwrap();
    let last_assistant_line = raw
        .lines()
        .filter(|l| l.contains("\"role\":\"assistant\""))
        .last()
        .expect("should have an assistant line");

    assert!(
        last_assistant_line.contains("claude-sonnet-4-6"),
        "model missing from last assistant line"
    );
    assert!(
        last_assistant_line.contains("openhuman-backend"),
        "provider missing from last assistant line"
    );
    assert!(
        last_assistant_line.contains("\"context_window\":200000"),
        "context window missing from usage"
    );
    assert!(
        last_assistant_line.contains("private reasoning trace"),
        "reasoning content missing from assistant metadata"
    );
    assert!(
        last_assistant_line.contains("\"tool_calls\""),
        "native tool calls missing from assistant metadata"
    );
    assert!(
        last_assistant_line.contains("\"cost_usd\""),
        "cost_usd missing"
    );

    // Messages themselves still round-trip byte-identically.
    let loaded = read_transcript(&path).unwrap();
    assert_eq!(loaded.messages.len(), messages.len());
    for (orig, got) in messages.iter().zip(loaded.messages.iter()) {
        assert_eq!(orig.role, got.role);
        assert_eq!(orig.content, got.content);
    }
}

#[test]
fn embedded_usage_preserves_earlier_assistant_messages_on_rewrite() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("multi_usage.jsonl");
    let mut messages = vec![
        ChatMessage::user("start"),
        ChatMessage::assistant("first"),
        ChatMessage::user("continue"),
        ChatMessage::assistant("second"),
    ];
    let first_usage = TurnUsage {
        provider: "provider-a".into(),
        model: "model-a".into(),
        iteration: 1,
        ..sample_turn_usage()
    };
    let second_usage = TurnUsage {
        provider: "provider-b".into(),
        model: "model-b".into(),
        iteration: 2,
        ..sample_turn_usage()
    };
    attach_turn_usage_metadata(&mut messages[1], &first_usage);

    write_transcript(&path, &messages, &sample_meta(), Some(&second_usage)).unwrap();

    let raw = fs::read_to_string(&path).unwrap();
    let assistant_lines: Vec<&str> = raw
        .lines()
        .filter(|line| line.contains("\"role\":\"assistant\""))
        .collect();
    assert_eq!(assistant_lines.len(), 2);
    assert!(assistant_lines[0].contains("provider-a"));
    assert!(assistant_lines[0].contains("model-a"));
    assert!(assistant_lines[1].contains("provider-b"));
    assert!(assistant_lines[1].contains("model-b"));

    let loaded = read_transcript(&path).unwrap();
    write_transcript(&path, &loaded.messages, &loaded.meta, None).unwrap();
    let rewritten = fs::read_to_string(&path).unwrap();
    assert!(rewritten.contains("provider-a"));
    assert!(rewritten.contains("model-a"));
    assert!(rewritten.contains("provider-b"));
    assert!(rewritten.contains("model-b"));
}

#[test]
fn md_companion_file_is_written() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("companion.jsonl");
    let messages = sample_messages();
    let meta = sample_meta();
    let tu = sample_turn_usage();

    write_transcript(&path, &messages, &meta, Some(&tu)).unwrap();

    let md_path = path.with_extension("md");
    assert!(md_path.exists(), ".md companion should be written");
    let md = fs::read_to_string(&md_path).unwrap();
    assert!(md.contains("# Session transcript — code_executor"));
    assert!(md.contains("Agent ID: `code_executor`"));
    assert!(md.contains("Agent type: `subagent`"));
    assert!(md.contains("Provider: `openhuman-backend`"));
    assert!(md.contains("Task: `task-123`"));
    assert!(
        md.contains("claude-sonnet-4-6"),
        "model should appear in md"
    );
    assert!(md.contains("private reasoning trace"));
    assert!(md.contains("## [system]"), "system section missing");
    assert!(md.contains("## [user]"), "user section missing");
}

#[test]
fn legacy_md_fallback_reads_old_session() {
    let dir = TempDir::new().unwrap();
    // Write a legacy .md file directly (old format).
    let md_path = dir.path().join("legacy.md");
    let legacy_content = "<!-- session_transcript\nagent: test_agent\ndispatcher: native\ncreated: 2026-01-01T00:00:00Z\nupdated: 2026-01-01T00:01:00Z\nturn_count: 1\ninput_tokens: 10\noutput_tokens: 5\ncached_input_tokens: 3\n-->\n\n<!--MSG role=\"system\"-->\nhello\n<!--/MSG-->\n";
    fs::write(&md_path, legacy_content).unwrap();

    // read_transcript called with a .jsonl path that doesn't exist
    // should fall back to the .md sibling.
    let jsonl_path = dir.path().join("legacy.jsonl");
    let loaded = read_transcript(&jsonl_path).unwrap();
    assert_eq!(loaded.meta.agent_name, "test_agent");
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].role, "system");
    assert_eq!(loaded.messages[0].content, "hello");
}

#[test]
fn unknown_fields_on_jsonl_lines_are_ignored() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("forward_compat.jsonl");

    // Write a JSONL with future unknown fields.
    let content = concat!(
        r#"{"_meta":{"agent":"a","dispatcher":"native","created":"t","updated":"t","turn_count":0,"input_tokens":0,"output_tokens":0,"cached_input_tokens":0,"charged_amount_usd":0.0}}"#,
        "\n",
        r#"{"role":"user","content":"hello","future_field":"ignored","another":42}"#,
        "\n"
    );
    fs::write(&path, content).unwrap();

    let loaded = read_transcript(&path).unwrap();
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].role, "user");
    assert_eq!(loaded.messages[0].content, "hello");
}

#[test]
fn next_index_counts_both_jsonl_and_md_files() {
    let dir = TempDir::new().unwrap();
    // Mix of legacy .md and new .jsonl for the same agent.
    fs::write(dir.path().join("main_0.md"), "legacy").unwrap();
    fs::write(dir.path().join("main_1.jsonl"), "new").unwrap();

    let next = next_index(dir.path(), "main").unwrap();
    assert_eq!(
        next, 2,
        "should account for both .md and .jsonl when computing next index"
    );
}

#[test]
fn latest_in_dir_prefers_jsonl_over_md() {
    let dir = TempDir::new().unwrap();
    // Same index: both .jsonl and .md exist — .jsonl should win.
    fs::write(dir.path().join("main_0.md"), "legacy").unwrap();
    fs::write(dir.path().join("main_0.jsonl"), "new").unwrap();

    let latest = latest_in_dir(dir.path(), "main").unwrap();
    assert!(
        latest.to_string_lossy().ends_with(".jsonl"),
        "should prefer .jsonl when both exist at same index"
    );
}

/// `thread_id` (the backend-side LLM thread identifier) must be both
/// emitted in the JSONL `_meta` header and surfaced in the `.md`
/// companion so a human reading the transcript can correlate it with
/// `InferenceLog` rows on the backend. Sessions without an ambient
/// thread (CLI, tests) keep `thread_id = None` and neither field
/// appears — the absence is intentional, not a missing feature.
#[test]
fn thread_id_round_trips_and_appears_in_md_when_present() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("thread.jsonl");
    let messages = sample_messages();
    let mut meta = sample_meta();
    meta.thread_id = Some("thread-xyz-42".into());

    write_transcript(&path, &messages, &meta, None).unwrap();

    // JSONL round-trip preserves the field.
    let loaded = read_transcript(&path).unwrap();
    assert_eq!(loaded.meta.thread_id.as_deref(), Some("thread-xyz-42"));

    // Markdown companion exposes it under the header.
    let md = fs::read_to_string(path.with_extension("md")).unwrap();
    assert!(
        md.contains("- Thread: `thread-xyz-42`"),
        "thread id should be rendered in md header, got:\n{md}"
    );
}

#[test]
fn thread_id_absent_omits_md_line_and_jsonl_field() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("no_thread.jsonl");
    let messages = sample_messages();
    let meta = sample_meta(); // thread_id = None

    write_transcript(&path, &messages, &meta, None).unwrap();

    let raw_jsonl = fs::read_to_string(&path).unwrap();
    assert!(
        !raw_jsonl.contains("\"thread_id\""),
        "absent thread_id must be skipped in JSONL so the field doesn't show up as `null`"
    );
    let md = fs::read_to_string(path.with_extension("md")).unwrap();
    assert!(
        !md.contains("- Thread:"),
        "no `- Thread:` line should appear when thread_id is None, got:\n{md}"
    );
}

// ── find_root_transcript_for_thread: scope isolation ────────────────────────

/// An empty or blank `thread_id` must not match any transcript — the
/// function should return `None` immediately rather than scan every JSONL
/// file looking for an empty `thread_id`.
#[test]
fn find_root_transcript_for_thread_returns_none_for_empty_thread_id() {
    let dir = TempDir::new().unwrap();
    let raw_dir = dir.path().join("session_raw");
    fs::create_dir_all(&raw_dir).unwrap();

    // Write a transcript that has a non-empty thread_id.
    let mut meta = sample_meta();
    meta.thread_id = Some("thread-abc".into());
    write_transcript(
        &raw_dir.join("1714000000_main.jsonl"),
        &sample_messages(),
        &meta,
        None,
    )
    .unwrap();

    assert!(
        find_root_transcript_for_thread(dir.path(), "").is_none(),
        "empty thread_id should return None"
    );
    assert!(
        find_root_transcript_for_thread(dir.path(), "   ").is_none(),
        "blank thread_id should return None"
    );
}

/// When two threads have transcripts in the same workspace, each call
/// must return **only** the file belonging to that thread — cross-thread
/// bleed must not occur.
#[test]
fn find_root_transcript_for_thread_isolates_by_thread_id() {
    let dir = TempDir::new().unwrap();
    let raw_dir = dir.path().join("session_raw");
    fs::create_dir_all(&raw_dir).unwrap();

    let mut meta_a = sample_meta();
    meta_a.thread_id = Some("thread-aaa".into());
    write_transcript(
        &raw_dir.join("1714000000_agent_thread-aaa.jsonl"),
        &sample_messages(),
        &meta_a,
        None,
    )
    .unwrap();

    let mut meta_b = sample_meta();
    meta_b.thread_id = Some("thread-bbb".into());
    write_transcript(
        &raw_dir.join("1714001000_agent_thread-bbb.jsonl"),
        &sample_messages(),
        &meta_b,
        None,
    )
    .unwrap();

    let found_a = find_root_transcript_for_thread(dir.path(), "thread-aaa")
        .expect("should find transcript for thread-aaa");
    let found_b = find_root_transcript_for_thread(dir.path(), "thread-bbb")
        .expect("should find transcript for thread-bbb");

    assert!(
        found_a
            .to_string_lossy()
            .contains("1714000000_agent_thread-aaa"),
        "wrong transcript returned for thread-aaa: {}",
        found_a.display()
    );
    assert!(
        found_b
            .to_string_lossy()
            .contains("1714001000_agent_thread-bbb"),
        "wrong transcript returned for thread-bbb: {}",
        found_b.display()
    );
}

/// `find_root_transcript_for_thread` returns the **newest** transcript
/// (highest stem, alphabetically) when multiple root files share the
/// same `thread_id`. This covers the agent restart scenario where a
/// session accumulates more than one transcript for the same thread.
#[test]
fn find_root_transcript_for_thread_returns_newest_when_multiple_match() {
    let dir = TempDir::new().unwrap();
    let raw_dir = dir.path().join("session_raw");
    fs::create_dir_all(&raw_dir).unwrap();

    let mut meta = sample_meta();
    meta.thread_id = Some("thread-multi".into());

    // Older file — lower timestamp.
    write_transcript(
        &raw_dir.join("1714000000_orchestrator_thread-multi.jsonl"),
        &sample_messages(),
        &meta,
        None,
    )
    .unwrap();

    // Newer file — higher timestamp; should be the one returned.
    write_transcript(
        &raw_dir.join("1715000000_orchestrator_thread-multi.jsonl"),
        &sample_messages(),
        &meta,
        None,
    )
    .unwrap();

    let found = find_root_transcript_for_thread(dir.path(), "thread-multi")
        .expect("should find newest transcript");
    assert!(
        found
            .to_string_lossy()
            .contains("1715000000_orchestrator_thread-multi"),
        "should return the newest transcript, got: {}",
        found.display()
    );
}

/// A subagent transcript (stem contains `__`) must be skipped even if
/// its `thread_id` matches — only root transcripts are eligible.
#[test]
fn find_root_transcript_for_thread_excludes_subagent_files() {
    let dir = TempDir::new().unwrap();
    let raw_dir = dir.path().join("session_raw");
    fs::create_dir_all(&raw_dir).unwrap();

    let mut meta = sample_meta();
    meta.thread_id = Some("thread-xyz".into());

    // Root transcript — should be found.
    write_transcript(
        &raw_dir.join("1714000000_orch_thread-xyz.jsonl"),
        &sample_messages(),
        &meta,
        None,
    )
    .unwrap();

    // Sub-agent transcript for the same thread — must be skipped.
    write_transcript(
        &raw_dir.join("1714000000_orch_thread-xyz__1714500000_worker.jsonl"),
        &sample_messages(),
        &meta,
        None,
    )
    .unwrap();

    let found = find_root_transcript_for_thread(dir.path(), "thread-xyz")
        .expect("should find the root transcript");
    let stem = found.file_stem().unwrap().to_string_lossy();
    assert!(
        !stem.contains("__"),
        "returned path must not be a subagent file (contains __): {}",
        found.display()
    );
}

#[test]
fn read_thread_usage_summary_totals_last_turn_and_model() {
    let ws = TempDir::new().unwrap();
    let raw = raw_session_dir(ws.path());
    std::fs::create_dir_all(&raw).unwrap();

    let mut meta = sample_meta();
    meta.thread_id = Some("thr-xyz".into());
    meta.input_tokens = 5000;
    meta.output_tokens = 1200;
    meta.cached_input_tokens = 800;
    meta.charged_amount_usd = 0.0045;
    meta.turn_count = 3;

    let tu = TurnUsage {
        provider: "openhuman-backend".into(),
        model: "reasoning-v1".into(),
        usage: MessageUsage {
            input: 400,
            output: 120,
            cached_input: 50,
            context_window: 1_000_000,
            cost_usd: 0.0009,
        },
        ts: "2026-04-11T14:35:22Z".into(),
        reasoning_content: None,
        tool_calls: Vec::new(),
        iteration: 0,
    };
    let path = raw.join("1700000000_main.jsonl");
    write_transcript(&path, &sample_messages(), &meta, Some(&tu)).unwrap();

    let summary = read_thread_usage_summary(ws.path(), "thr-xyz").expect("summary present");
    assert_eq!(summary.input_tokens, 5000);
    assert_eq!(summary.output_tokens, 1200);
    assert_eq!(summary.cached_input_tokens, 800);
    assert!((summary.cost_usd - 0.0045).abs() < 1e-9);
    assert_eq!(summary.turn_count, 3);
    assert_eq!(summary.last_turn_input_tokens, 400);
    assert_eq!(summary.last_turn_output_tokens, 120);
    assert_eq!(summary.model.as_deref(), Some("reasoning-v1"));
}

#[test]
fn read_thread_usage_summary_sums_multiple_transcripts() {
    let ws = TempDir::new().unwrap();
    let raw = raw_session_dir(ws.path());
    std::fs::create_dir_all(&raw).unwrap();

    let mk = |stem: &str, input: u64, cost: f64| {
        let mut meta = sample_meta();
        meta.thread_id = Some("thr-multi".into());
        meta.input_tokens = input;
        meta.output_tokens = 0;
        meta.cached_input_tokens = 0;
        meta.charged_amount_usd = cost;
        meta.turn_count = 1;
        write_transcript(
            &raw.join(format!("{stem}.jsonl")),
            &sample_messages(),
            &meta,
            None,
        )
        .unwrap();
    };
    mk("1700000000_main", 100, 0.01);
    mk("1700000100_main", 250, 0.02);

    let s = read_thread_usage_summary(ws.path(), "thr-multi").expect("summary present");
    assert_eq!(s.input_tokens, 350);
    assert!((s.cost_usd - 0.03).abs() < 1e-9);
    assert_eq!(s.turn_count, 2);
}

#[test]
fn read_thread_usage_summary_none_for_unknown_thread() {
    let ws = TempDir::new().unwrap();
    assert!(read_thread_usage_summary(ws.path(), "no-such-thread").is_none());
    // Empty thread id is rejected too.
    assert!(read_thread_usage_summary(ws.path(), "   ").is_none());
}

#[test]
fn read_thread_usage_summary_groups_subagents_by_archetype() {
    let ws = TempDir::new().unwrap();
    let raw = raw_session_dir(ws.path());
    std::fs::create_dir_all(&raw).unwrap();

    // Root (orchestrator) transcript — never includes sub-agent calls.
    let mut root = sample_meta();
    root.thread_id = Some("thr-sub".into());
    root.agent_name = "main".into();
    root.input_tokens = 1000;
    root.output_tokens = 200;
    root.cached_input_tokens = 0;
    root.charged_amount_usd = 0.0;
    root.turn_count = 2;
    write_transcript(
        &raw.join("1700000000_main.jsonl"),
        &sample_messages(),
        &root,
        None,
    )
    .unwrap();

    // Sub-agent transcripts (stems contain `__`): coder x2 + researcher x1.
    let sub = |stem: &str, agent: &str, input: u64, output: u64| {
        let mut m = sample_meta();
        m.thread_id = Some("thr-sub".into());
        m.agent_name = agent.into();
        m.input_tokens = input;
        m.output_tokens = output;
        m.cached_input_tokens = 0;
        m.charged_amount_usd = 0.0;
        m.turn_count = 1;
        write_transcript(
            &raw.join(format!("{stem}.jsonl")),
            &sample_messages(),
            &m,
            None,
        )
        .unwrap();
    };
    sub("1700000000_main__1700000001_coder", "coder", 300, 60);
    sub("1700000000_main__1700000002_coder", "coder", 100, 20);
    sub(
        "1700000000_main__1700000003_researcher",
        "researcher",
        500,
        90,
    );

    let s = read_thread_usage_summary(ws.path(), "thr-sub").expect("summary present");
    // Root totals are orchestrator-only (sub-agents are separate).
    assert_eq!(s.input_tokens, 1000);
    assert_eq!(s.output_tokens, 200);
    // Grouped by archetype.
    assert_eq!(s.subagents.len(), 2);
    let coder = s
        .subagents
        .iter()
        .find(|g| g.agent_id == "coder")
        .expect("coder group");
    assert_eq!(coder.input_tokens, 400);
    assert_eq!(coder.output_tokens, 80);
    assert_eq!(coder.runs, 2);
    let researcher = s
        .subagents
        .iter()
        .find(|g| g.agent_id == "researcher")
        .expect("researcher group");
    assert_eq!(researcher.input_tokens, 500);
    assert_eq!(researcher.runs, 1);
}

// ── Phase A: append-only + compaction + display + interrupted ─────────

/// A helper mirroring the in-process persist loop: track the previously
/// persisted logical set and feed each turn through `append_transcript_turn`.
struct AppendHarness {
    path: std::path::PathBuf,
    prev: Vec<ChatMessage>,
}

impl AppendHarness {
    fn new(path: std::path::PathBuf) -> Self {
        Self {
            path,
            prev: Vec::new(),
        }
    }

    fn turn(
        &mut self,
        messages: &[ChatMessage],
        meta: &TranscriptMeta,
        usage: Option<&TurnUsage>,
        request_id: Option<&str>,
    ) {
        append_transcript_turn(&self.path, &self.prev, messages, meta, usage, request_id)
            .expect("append turn");
        self.prev = messages.to_vec();
    }
}

fn roles(messages: &[ChatMessage]) -> Vec<&str> {
    messages.iter().map(|m| m.role.as_str()).collect()
}

/// Pure extension across turns: the model-context read reflects the final
/// (growing) message set and never rewrites earlier lines.
#[test]
fn append_pure_extension_grows_context() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("append.jsonl");
    let meta = sample_meta();
    let mut h = AppendHarness::new(path.clone());

    let turn1 = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("hi"),
        ChatMessage::assistant("hello"),
    ];
    h.turn(&turn1, &meta, None, None);

    let mut turn2 = turn1.clone();
    turn2.push(ChatMessage::user("again"));
    turn2.push(ChatMessage::assistant("hello again"));
    h.turn(&turn2, &meta, None, None);

    let loaded = read_transcript(&path).unwrap();
    assert_eq!(
        roles(&loaded.messages),
        vec!["system", "user", "assistant", "user", "assistant"]
    );
    assert_eq!(loaded.messages[4].content, "hello again");
}

/// Compaction round-trip: after a reduction, the model-context read returns the
/// REDUCED context, while the display read returns the FULL pre-compaction
/// history plus the compaction marker.
#[test]
fn compaction_round_trip_model_vs_display() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("compact.jsonl");
    let meta = sample_meta();
    let mut h = AppendHarness::new(path.clone());

    // Three growing turns.
    let base = vec![
        ChatMessage::system("sys"),
        ChatMessage::user("q1"),
        ChatMessage::assistant("a1"),
        ChatMessage::user("q2"),
        ChatMessage::assistant("a2"),
    ];
    h.turn(&base, &meta, None, None);

    // A reduction: the harness drops the earliest exchange and keeps a summary
    // + the recent tail. This is NOT a prefix of `base`, so it must land as a
    // compaction record.
    let reduced = vec![
        ChatMessage::system("sys"),
        ChatMessage::assistant("[summary] earlier discussion about q1/q2"),
        ChatMessage::user("q3"),
        ChatMessage::assistant("a3"),
    ];
    h.turn(&reduced, &meta, None, None);

    // Model-context read == the reduced set only.
    let model = read_transcript(&path).unwrap();
    assert_eq!(
        model
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        vec![
            "sys",
            "[summary] earlier discussion about q1/q2",
            "q3",
            "a3"
        ],
        "model context must reflect the reduced set after compaction"
    );

    // Display read == full history: the 5 pre-compaction messages, then a
    // compaction marker carrying the 4-message replacement.
    let display = read_transcript_display(&path).unwrap();
    let pre: Vec<&str> = display
        .records
        .iter()
        .take_while(|r| matches!(r, DisplayRecord::Message(_)))
        .filter_map(|r| match r {
            DisplayRecord::Message(m) => Some(m.message.content.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(pre, vec!["sys", "q1", "a1", "q2", "a2"]);
    let marker = display
        .records
        .iter()
        .find_map(|r| match r {
            DisplayRecord::Compaction(c) => Some(c),
            _ => None,
        })
        .expect("display must retain the compaction marker");
    assert_eq!(marker.replacement.len(), 4);
    assert_eq!(
        marker.replacement[1].message.content,
        "[summary] earlier discussion about q1/q2"
    );
}

/// After a compaction, a subsequent pure extension appends normally and the
/// model-context read replays reset-then-extend to the correct final set.
#[test]
fn append_after_compaction_extends_reduced_set() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("compact_then_extend.jsonl");
    let meta = sample_meta();
    let mut h = AppendHarness::new(path.clone());

    h.turn(
        &[
            ChatMessage::system("sys"),
            ChatMessage::user("q1"),
            ChatMessage::assistant("a1"),
        ],
        &meta,
        None,
        None,
    );
    let reduced = vec![
        ChatMessage::system("sys"),
        ChatMessage::assistant("[summary]"),
    ];
    h.turn(&reduced, &meta, None, None);
    let mut extended = reduced.clone();
    extended.push(ChatMessage::user("q2"));
    extended.push(ChatMessage::assistant("a2"));
    h.turn(&extended, &meta, None, None);

    let model = read_transcript(&path).unwrap();
    assert_eq!(
        model
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>(),
        vec!["sys", "[summary]", "q2", "a2"]
    );
}

/// request_id turn-boundary stamping round-trips into the display projection on
/// every appended line of a turn.
#[test]
fn request_id_stamped_on_every_line() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("reqid.jsonl");
    let meta = sample_meta();
    let mut h = AppendHarness::new(path.clone());

    let turn1 = vec![ChatMessage::system("sys"), ChatMessage::user("q1")];
    h.turn(&turn1, &meta, None, Some("req-1"));

    let mut turn2 = turn1.clone();
    turn2.push(ChatMessage::assistant("a2"));
    h.turn(&turn2, &meta, None, Some("req-2"));

    let display = read_transcript_display(&path).unwrap();
    let msgs: Vec<&DisplayMessage> = display
        .records
        .iter()
        .filter_map(|r| match r {
            DisplayRecord::Message(m) => Some(m),
            _ => None,
        })
        .collect();
    // turn1 wrote sys + user with req-1; turn2 appended only the assistant tail
    // with req-2.
    assert_eq!(msgs[0].request_id.as_deref(), Some("req-1"));
    assert_eq!(msgs[1].request_id.as_deref(), Some("req-1"));
    assert_eq!(msgs[2].request_id.as_deref(), Some("req-2"));
    assert_eq!(msgs[2].message.content, "a2");
}

/// An interrupted partial is appended to the file, is visible in the display
/// read flagged `interrupted`, and is SKIPPED by the model-context read (a
/// resumed context never carries a truncated answer).
#[test]
fn interrupted_partial_display_only() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("interrupted.jsonl");
    let meta = sample_meta();
    let mut h = AppendHarness::new(path.clone());
    h.turn(
        &[ChatMessage::system("sys"), ChatMessage::user("q1")],
        &meta,
        None,
        Some("req-1"),
    );

    append_interrupted_partial(
        &path,
        "partial answer that was cut off",
        Some("req-1"),
        Some(3),
        Some("thinking that was cut off"),
    )
    .expect("append interrupted");

    // Model context: the partial is skipped.
    let model = read_transcript(&path).unwrap();
    assert_eq!(roles(&model.messages), vec!["system", "user"]);
    assert!(
        !model.messages.iter().any(|m| m.content.contains("cut off")),
        "interrupted partial must NOT enter the model context"
    );

    // Display: the partial is present and flagged.
    let display = read_transcript_display(&path).unwrap();
    let partial = display
        .records
        .iter()
        .find_map(|r| match r {
            DisplayRecord::Message(m) if m.interrupted => Some(m),
            _ => None,
        })
        .expect("display must include the interrupted partial");
    assert_eq!(partial.message.content, "partial answer that was cut off");
    assert_eq!(partial.request_id.as_deref(), Some("req-1"));
    assert_eq!(partial.iteration, Some(3));
    assert_eq!(
        partial.reasoning_content.as_deref(),
        Some("thinking that was cut off"),
        "interrupted partial must carry its reasoning_content"
    );
}

/// Empty partial content is a no-op — no line is written.
#[test]
fn interrupted_partial_empty_is_noop() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty_interrupt.jsonl");
    let meta = sample_meta();
    let mut h = AppendHarness::new(path.clone());
    h.turn(&[ChatMessage::user("q")], &meta, None, None);
    append_interrupted_partial(&path, "", None, None, None).expect("noop");
    let display = read_transcript_display(&path).unwrap();
    assert!(display
        .records
        .iter()
        .all(|r| matches!(r, DisplayRecord::Message(m) if !m.interrupted)));
}

/// A legacy file — one produced by the full-rewrite `write_transcript` with no
/// compaction records and no `version` — reads identically under both the
/// model-context and display readers.
#[test]
fn legacy_file_reads_identically() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("legacy.jsonl");
    let messages = sample_messages();
    let meta = sample_meta();
    // Full-rewrite writer == the legacy shape (append-only readers must tolerate
    // it: zero compaction records, last `_meta` == the only `_meta`).
    write_transcript(&path, &messages, &meta, None).unwrap();

    let model = read_transcript(&path).unwrap();
    assert_eq!(model.messages.len(), messages.len());
    assert_eq!(roles(&model.messages), roles(&messages));

    let display = read_transcript_display(&path).unwrap();
    let display_roles: Vec<&str> = display
        .records
        .iter()
        .filter_map(|r| match r {
            DisplayRecord::Message(m) => Some(m.message.role.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(display_roles, roles(&messages));
}

/// A file carrying an unknown record kind (as a future core might write) is
/// skipped by the reader rather than crashing it.
#[test]
fn unknown_record_kind_is_skipped_not_fatal() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("unknown_kind.jsonl");
    let meta = sample_meta();
    let mut h = AppendHarness::new(path.clone());
    h.turn(
        &[ChatMessage::system("sys"), ChatMessage::user("q1")],
        &meta,
        None,
        None,
    );
    // Simulate a future kind by appending a foreign record line.
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "{{\"kind\":\"future_thing\",\"payload\":42}}").unwrap();
    }
    // Append a normal turn after the unknown line to prove reading continues.
    let mut msgs = vec![ChatMessage::system("sys"), ChatMessage::user("q1")];
    msgs.push(ChatMessage::assistant("a1"));
    h.prev = vec![ChatMessage::system("sys"), ChatMessage::user("q1")];
    h.turn(&msgs, &meta, None, None);

    let model = read_transcript(&path).unwrap();
    // The unknown record is skipped; the real messages survive.
    assert!(model.messages.iter().any(|m| m.content == "a1"));
    assert!(!model
        .messages
        .iter()
        .any(|m| m.content.contains("future_thing")));
}

/// The `_meta` version field is stamped by the append writer and absent (0) on
/// legacy files — but both remain readable.
#[test]
fn meta_version_stamped_and_optional() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("version.jsonl");
    let meta = sample_meta();
    let mut h = AppendHarness::new(path.clone());
    h.turn(&[ChatMessage::user("q")], &meta, None, None);
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(
        raw.lines().next().unwrap().contains("\"version\":1"),
        "append writer must stamp the schema version on the meta header"
    );
}
