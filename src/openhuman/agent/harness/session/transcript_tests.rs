use super::*;
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
        dispatcher: "native".into(),
        created: "2026-04-11T14:30:00Z".into(),
        updated: "2026-04-11T14:35:22Z".into(),
        turn_count: 3,
        input_tokens: 5000,
        output_tokens: 1200,
        cached_input_tokens: 3500,
        charged_amount_usd: 0.0045,
    }
}

fn sample_turn_usage() -> TurnUsage {
    TurnUsage {
        model: "claude-sonnet-4-6".into(),
        usage: MessageUsage {
            input: 1234,
            output: 567,
            cached_input: 1000,
            cost_usd: 0.0012,
        },
        ts: "2026-04-17T10:00:00Z".into(),
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
        assert_eq!(original.role, loaded.role, "role mismatch");
        assert_eq!(
            original.content, loaded.content,
            "content mismatch for role={}",
            original.role
        );
    }
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
        PathBuf::from(format!(
            "/tmp/ws/sessions/{today}/1714000000_main.md"
        )),
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
    assert!(latest
        .to_string_lossy()
        .ends_with("1714999999_main.jsonl"));
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
    assert!(
        md.contains("claude-sonnet-4-6"),
        "model should appear in md"
    );
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
