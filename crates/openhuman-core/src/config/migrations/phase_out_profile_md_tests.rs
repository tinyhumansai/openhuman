use super::*;
use crate::util::classic_transcript_fixture::{read_classic_rows, write_classic};
use std::fs;
use tempfile::TempDir;
use tinyagents_session::transcript::TranscriptMessage;
use tinyagents_session::transcript::TranscriptMeta;

fn durable_messages(
    messages: impl IntoIterator<Item = TranscriptMessage>,
) -> Vec<tinyagents_session::transcript::TranscriptMessage> {
    messages.into_iter().collect()
}

fn meta() -> TranscriptMeta {
    TranscriptMeta {
        session_id: None,
        parent_session_id: None,
        agent_name: "main".into(),
        agent_id: None,
        agent_type: None,
        dispatcher: "native".into(),
        provider: None,
        model: None,
        created: "2026-05-01T00:00:00Z".into(),
        updated: "2026-05-01T00:00:00Z".into(),
        turn_count: 1,
        prefix_message_count: None,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: None,
        task_id: None,
    }
}

fn write_tainted_transcript(workspace_dir: &Path, stem: &str, system_body: &str) -> PathBuf {
    let raw_dir = workspace_dir.join("session_raw");
    fs::create_dir_all(&raw_dir).unwrap();
    let path = raw_dir.join(format!("{stem}.jsonl"));
    let messages = vec![
        TranscriptMessage::system(system_body),
        TranscriptMessage::user("hello"),
        TranscriptMessage::assistant("hi"),
    ];
    write_classic(&path, &durable_messages(messages), &meta());
    path
}

fn prompt_with_profile_block(profile_body: &str) -> String {
    format!(
        "## Identity\n\nYou are an assistant.\n\n\
         ### PROFILE.md\n\n{profile_body}\n\n\
         ### Tools\n\n- shell\n"
    )
}

fn prompt_with_profile_block_at_end(profile_body: &str) -> String {
    format!(
        "## Identity\n\nYou are an assistant.\n\n\
         ### PROFILE.md\n\n{profile_body}\n"
    )
}

fn prompt_with_truncated_profile_block() -> String {
    "## Identity\n\nYou are an assistant.\n\n\
     ### PROFILE.md\n\n\
     style/calm tooling/rust vetoes/none goals/ship\n\n\
     [... truncated at 4000 chars — use `read` for full file]\n\n\
     ### Tools\n\n- shell\n"
        .to_string()
}

#[test]
fn strip_block_terminated_by_next_section() {
    let prompt = prompt_with_profile_block("style/calm tooling/rust");
    let cleaned = strip_profile_md_block(&prompt).unwrap();
    assert!(
        !cleaned.contains("### PROFILE.md"),
        "PROFILE.md heading must be gone, got:\n{cleaned}"
    );
    assert!(
        !cleaned.contains("style/calm tooling/rust"),
        "PROFILE.md body must be gone, got:\n{cleaned}"
    );
    assert!(
        cleaned.contains("## Identity"),
        "earlier content must survive, got:\n{cleaned}"
    );
    assert!(
        cleaned.contains("### Tools"),
        "next section must survive, got:\n{cleaned}"
    );
}

#[test]
fn strip_block_at_end_of_prompt() {
    let prompt = prompt_with_profile_block_at_end("style/calm tooling/rust");
    let cleaned = strip_profile_md_block(&prompt).unwrap();
    assert!(!cleaned.contains("### PROFILE.md"));
    assert!(!cleaned.contains("style/calm tooling/rust"));
    assert!(cleaned.contains("## Identity"));
    assert!(
        !cleaned.ends_with("\n\n\n"),
        "trailing whitespace must be tidied, got:\n{cleaned:?}"
    );
}

#[test]
fn strip_block_handles_truncation_footer() {
    let prompt = prompt_with_truncated_profile_block();
    let cleaned = strip_profile_md_block(&prompt).unwrap();
    assert!(!cleaned.contains("### PROFILE.md"));
    assert!(!cleaned.contains("style/calm"));
    assert!(
        !cleaned.contains("[... truncated at"),
        "truncation footer must be removed too, got:\n{cleaned}"
    );
    assert!(cleaned.contains("### Tools"));
}

#[test]
fn strip_block_returns_none_when_absent() {
    let prompt = "## Identity\n\nYou are an assistant.\n\n### Tools\n\n- shell\n";
    assert!(strip_profile_md_block(prompt).is_none());
}

#[test]
fn strip_block_returns_none_when_reconstruction_is_byte_identical() {
    // Edge case: a heading line followed immediately by a boundary
    // marker on the next line, with no surrounding whitespace to
    // collapse. Once the block is excised + whitespace tidied, the
    // output could match the input byte-for-byte; the stripper must
    // detect that and return None so callers don't rewrite the file.
    let prompt = "head\n### PROFILE.md\n### Tools\nrest\n";
    let cleaned = strip_profile_md_block(prompt);
    // Verify: either the stripper returned None (no-change short-
    // circuit) OR it produced a string that's actually different.
    if let Some(out) = cleaned.as_deref() {
        assert_ne!(out, prompt, "if Some is returned the bytes must differ");
    }
}

#[test]
fn strip_block_does_not_match_substring_inside_other_content() {
    // A line that *mentions* "### PROFILE.md" inline (not as a heading)
    // should not anchor the strip.
    let prompt = "## Notes\n\nSee `### PROFILE.md` referenced inline.\n\n### Tools\n\n- shell\n";
    assert!(strip_profile_md_block(prompt).is_none());
}

#[test]
fn sanitize_only_touches_first_system_message() {
    let dir = TempDir::new().unwrap();
    let path = write_tainted_transcript(
        dir.path(),
        "1700000000_main",
        &prompt_with_profile_block("style/calm"),
    );
    // Add a later user message that also mentions PROFILE.md — it must
    // survive the migration unchanged.
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(
        &serde_json::to_string(&TranscriptMessage::user(
            "Could you show me what was in ### PROFILE.md earlier?",
        ))
        .unwrap(),
    );
    text.push('\n');
    fs::write(&path, text).unwrap();

    let stats = run(dir.path()).unwrap();
    assert_eq!(
        stats.cleaned, 1,
        "first system message had a block, must mutate"
    );

    let rows = read_classic_rows(&path);
    assert!(!rows[0]["content"]
        .as_str()
        .unwrap()
        .contains("### PROFILE.md"));
    assert!(rows.iter().any(|m| {
        m["role"] == "user" && m["content"].as_str().unwrap().contains("### PROFILE.md")
    }));
}

#[test]
fn run_cleans_flat_and_legacy_dirs_in_one_pass() {
    let dir = TempDir::new().unwrap();

    // Flat layout: session_raw/{stem}.jsonl
    write_tainted_transcript(
        dir.path(),
        "1700000000_main",
        &prompt_with_profile_block("style/calm"),
    );

    // Legacy layout: session_raw/DDMMYYYY/{stem}.jsonl
    let legacy_dir = dir.path().join("session_raw").join("12042025");
    fs::create_dir_all(&legacy_dir).unwrap();
    let legacy_path = legacy_dir.join("1700000001_main.jsonl");
    let messages = vec![
        TranscriptMessage::system(prompt_with_profile_block("style/legacy")),
        TranscriptMessage::user("legacy"),
    ];
    write_classic(&legacy_path, &durable_messages(messages), &meta());

    let stats = run(dir.path()).unwrap();
    assert_eq!(stats.scanned, 2);
    assert_eq!(stats.cleaned, 2);
    assert_eq!(stats.skipped, 0);
    assert_eq!(stats.errors, 0);
}

#[test]
fn run_short_circuits_on_fresh_install() {
    let dir = TempDir::new().unwrap();
    // session_raw/ does not exist — fresh install. PROFILE.md also absent.
    let stats = run(dir.path()).unwrap();
    assert_eq!(stats, PhaseOutStats::default());

    // session_raw/ exists but holds no .jsonl files.
    fs::create_dir_all(dir.path().join("session_raw")).unwrap();
    let stats = run(dir.path()).unwrap();
    assert_eq!(stats, PhaseOutStats::default());
}

#[test]
fn run_removes_workspace_profile_md() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path()).unwrap();
    let profile_md = dir.path().join("PROFILE.md");
    fs::write(&profile_md, "style/calm tooling/rust").unwrap();

    let stats = run(dir.path()).unwrap();
    assert!(stats.profile_md_removed, "PROFILE.md must be removed");
    assert!(!profile_md.exists(), "file must be gone after migration");
    // No transcripts present → other counters stay zero.
    assert_eq!(stats.scanned, 0);
    assert_eq!(stats.cleaned, 0);
    assert_eq!(stats.errors, 0);
}

#[test]
fn run_removes_profile_md_alongside_transcript_cleanup() {
    let dir = TempDir::new().unwrap();
    let profile_md = dir.path().join("PROFILE.md");
    fs::write(&profile_md, "style/calm").unwrap();
    write_tainted_transcript(
        dir.path(),
        "1700000000_main",
        &prompt_with_profile_block("style/calm"),
    );

    let stats = run(dir.path()).unwrap();
    assert!(stats.profile_md_removed);
    assert!(!profile_md.exists());
    assert_eq!(stats.cleaned, 1);
    assert_eq!(stats.scanned, 1);
}

#[test]
fn run_strips_profile_block_from_new_format_md_in_place() {
    let dir = TempDir::new().unwrap();
    let dated = dir.path().join("sessions").join("2026_05_14");
    fs::create_dir_all(&dated).unwrap();
    let md = dated.join("1700000000_orchestrator.md");
    // New-format companion: system message contains a PROFILE.md block
    // followed by a `---` separator and a `## [user]` message that
    // MUST survive intact (the proof is preserved).
    let body = "# Session transcript\n\n---\n\n## [system]\n\n\
                Identity preamble.\n\n\
                ### PROFILE.md\n\nstyle/calm tooling/rust\n\n\
                ---\n\n\
                ## [user]\n\nHello there.\n";
    fs::write(&md, body).unwrap();

    let stats = run(dir.path()).unwrap();
    assert_eq!(stats.md_companions_altered, 1);
    assert!(md.exists(), "file must still exist — proof preserved");
    let cleaned = fs::read_to_string(&md).unwrap();
    assert!(!cleaned.contains("### PROFILE.md"));
    assert!(!cleaned.contains("style/calm tooling/rust"));
    assert!(cleaned.contains("Identity preamble."));
    assert!(
        cleaned.contains("## [user]\n\nHello there."),
        "user message must survive:\n{cleaned}"
    );
}

#[test]
fn run_strips_profile_block_from_legacy_html_md_in_place() {
    let dir = TempDir::new().unwrap();
    let legacy_dir = dir.path().join("sessions").join("17042026");
    fs::create_dir_all(&legacy_dir).unwrap();
    let md = legacy_dir.join("orchestrator_thread-123.md");
    let body = "<!-- session_transcript\nagent: orchestrator\n-->\n\
                <!--MSG role=\"system\"-->\n\
                Identity preamble.\n\n\
                ### PROFILE.md\n\nstyle/calm\n\
                <!--/MSG-->\n\
                <!--MSG role=\"user\"-->\n\
                Hello.\n\
                <!--/MSG-->\n";
    fs::write(&md, body).unwrap();

    let stats = run(dir.path()).unwrap();
    assert_eq!(stats.md_companions_altered, 1);
    assert!(md.exists());
    let cleaned = fs::read_to_string(&md).unwrap();
    assert!(!cleaned.contains("### PROFILE.md"));
    assert!(!cleaned.contains("style/calm"));
    assert!(cleaned.contains("Identity preamble."));
    assert!(
        cleaned.contains("<!--MSG role=\"user\"-->\nHello."),
        "later user message must survive:\n{cleaned}"
    );
}

#[test]
fn run_leaves_clean_md_files_byte_identical() {
    let dir = TempDir::new().unwrap();
    let dated = dir.path().join("sessions").join("2026_05_14");
    fs::create_dir_all(&dated).unwrap();
    let md = dated.join("clean.md");
    let body = "## [system]\n\nNo profile here.\n\n---\n\n## [user]\n\nhi\n";
    fs::write(&md, body).unwrap();
    let before = fs::read(&md).unwrap();

    let stats = run(dir.path()).unwrap();
    assert_eq!(stats.md_companions_altered, 0);
    let after = fs::read(&md).unwrap();
    assert_eq!(before, after, "clean md must be byte-identical");
}

#[test]
fn run_md_sweep_short_circuits_when_sessions_dir_missing() {
    let dir = TempDir::new().unwrap();
    let stats = run(dir.path()).unwrap();
    assert_eq!(stats.md_companions_altered, 0);
}

#[test]
fn run_profile_md_removal_is_idempotent() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("PROFILE.md"), "x").unwrap();

    let first = run(dir.path()).unwrap();
    assert!(first.profile_md_removed);

    let second = run(dir.path()).unwrap();
    assert!(
        !second.profile_md_removed,
        "second run sees no file, must report false"
    );
}

#[test]
fn run_is_idempotent() {
    let dir = TempDir::new().unwrap();
    write_tainted_transcript(
        dir.path(),
        "1700000000_main",
        &prompt_with_profile_block("style/calm"),
    );

    let first = run(dir.path()).unwrap();
    assert_eq!(first.cleaned, 1);
    assert_eq!(first.skipped, 0);

    let second = run(dir.path()).unwrap();
    assert_eq!(second.cleaned, 0);
    assert_eq!(second.skipped, 1, "second run must see file as clean");
}

#[test]
fn run_leaves_clean_transcripts_byte_identical() {
    let dir = TempDir::new().unwrap();
    let raw_dir = dir.path().join("session_raw");
    fs::create_dir_all(&raw_dir).unwrap();
    let path = raw_dir.join("1700000000_main.jsonl");
    let messages = vec![
        TranscriptMessage::system("## Identity\n\nNo profile here.\n\n### Tools\n\n- shell\n"),
        TranscriptMessage::user("hi"),
    ];
    write_classic(&path, &durable_messages(messages), &meta());
    let before = fs::read(&path).unwrap();

    let stats = run(dir.path()).unwrap();
    assert_eq!(stats.cleaned, 0);
    assert_eq!(stats.skipped, 1);

    let after = fs::read(&path).unwrap();
    assert_eq!(before, after, "clean transcript must be byte-identical");
}

#[test]
fn run_ignores_non_jsonl_files_in_session_raw() {
    let dir = TempDir::new().unwrap();
    let raw_dir = dir.path().join("session_raw");
    fs::create_dir_all(&raw_dir).unwrap();
    fs::write(raw_dir.join("notes.md"), "hello").unwrap();
    fs::write(raw_dir.join("config.toml"), "[x]\ny = 1").unwrap();

    let stats = run(dir.path()).unwrap();
    assert_eq!(stats, PhaseOutStats::default());
}
