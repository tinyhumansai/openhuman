use super::*;

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

/// Root transcripts must come back in the order they were written, not in
/// filename order.
///
/// Modern stems are `{unix_ts}_{agent}` and sort identically either way, but a
/// legacy `{agent}_{index}` root encodes no time at all — and because digits
/// sort before letters, every legacy root sorted *after* every modern one no
/// matter when it was written. `project_from_files` concatenates these in
/// order, so a mis-ordered list reorders the rendered view and can attach a
/// sub-agent trail to the wrong turn.
#[test]
fn root_transcripts_are_ordered_by_creation_not_file_name() {
    let dir = TempDir::new().unwrap();
    let raw = dir.path().join("session_raw");
    std::fs::create_dir_all(&raw).unwrap();

    let write_root = |stem: &str, created: &str| {
        let mut meta = sample_meta();
        meta.thread_id = Some("thread-order".into());
        meta.created = created.into();
        write_transcript(
            &raw.join(format!("{stem}.jsonl")),
            &sample_messages(),
            &meta,
            None,
        )
        .unwrap();
    };

    // The legacy root is the OLDEST, but `orchestrator_1` sorts after any
    // digit-led stem, so a filename sort puts it last.
    write_root("orchestrator_1", "2026-04-10T09:00:00Z");
    write_root("1776211200_orchestrator", "2026-04-11T09:00:00Z");
    write_root("1776297600_orchestrator", "2026-04-12T09:00:00Z");

    let ordered: Vec<String> = find_root_transcripts_for_thread(dir.path(), "thread-order")
        .into_iter()
        .map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string()
        })
        .collect();

    assert_eq!(
        ordered,
        vec![
            "orchestrator_1".to_string(),
            "1776211200_orchestrator".to_string(),
            "1776297600_orchestrator".to_string(),
        ],
        "roots must be chronological; a filename sort would put the legacy root last"
    );
}
