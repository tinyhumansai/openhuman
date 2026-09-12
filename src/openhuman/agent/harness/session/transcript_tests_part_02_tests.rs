use super::*;

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
fn read_thread_usage_summary_scans_profile_scoped_raw_dirs() {
    let ws = TempDir::new().unwrap();
    let raw = ws.path().join("session_raw-alice");
    std::fs::create_dir_all(&raw).unwrap();
    let mut meta = sample_meta();
    meta.thread_id = Some("thr-scoped-usage".into());
    meta.input_tokens = 321;
    meta.output_tokens = 45;
    meta.turn_count = 2;
    write_transcript(
        &raw.join("1700000000_main.jsonl"),
        &sample_messages(),
        &meta,
        None,
    )
    .unwrap();

    let summary = read_thread_usage_summary(ws.path(), "thr-scoped-usage")
        .expect("scoped usage summary present");
    assert_eq!(summary.input_tokens, 321);
    assert_eq!(summary.output_tokens, 45);
    assert_eq!(summary.turn_count, 2);
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
