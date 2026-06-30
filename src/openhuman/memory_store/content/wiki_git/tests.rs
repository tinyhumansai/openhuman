use super::*;
use git2::IndexAddOption;
use tempfile::TempDir;

#[test]
fn commit_summary_initializes_repo_and_tracks_only_summaries() {
    let dir = TempDir::new().unwrap();
    let wiki = dir.path().join("wiki");
    let summary = wiki.join("summaries/source-slack/L1/summary-1.md");
    let raw = wiki.join("raw/should-not-track.md");
    let note = wiki.join("notes/also-ignored.md");
    std::fs::create_dir_all(summary.parent().unwrap()).unwrap();
    std::fs::create_dir_all(raw.parent().unwrap()).unwrap();
    std::fs::create_dir_all(note.parent().unwrap()).unwrap();
    std::fs::write(&summary, "---\nkind: summary\n---\nbody").unwrap();
    std::fs::write(&raw, "raw").unwrap();
    std::fs::write(&note, "note").unwrap();

    commit_summaries(
        dir.path(),
        &batch(
            "queued_seal",
            vec![entry(
                "summary-1",
                "wiki/summaries/source-slack/L1/summary-1.md",
            )],
        ),
    )
    .unwrap();

    let repo = Repository::open(&wiki).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let tree = head.tree().unwrap();
    assert!(tree.get_path(Path::new(".gitignore")).is_ok());
    assert!(tree
        .get_path(Path::new("summaries/source-slack/L1/summary-1.md"))
        .is_ok());
    assert!(tree.get_path(Path::new("raw/should-not-track.md")).is_err());
    assert!(tree.get_path(Path::new("notes/also-ignored.md")).is_err());
}

#[test]
fn commit_summary_prunes_existing_non_summary_tracked_entries() {
    let dir = TempDir::new().unwrap();
    let wiki = dir.path().join("wiki");
    std::fs::create_dir_all(wiki.join("raw")).unwrap();
    std::fs::create_dir_all(wiki.join("summaries/source/L1")).unwrap();
    std::fs::write(wiki.join("raw/old.md"), "old raw").unwrap();
    std::fs::write(wiki.join("summaries/source/L1/new.md"), "new summary").unwrap();

    let repo = Repository::init(&wiki).unwrap();
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"].iter(), IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree_oid = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();
    let sig = Signature::now(SIG_NAME, SIG_EMAIL).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "old mixed commit", &tree, &[])
        .unwrap();

    commit_summaries(
        dir.path(),
        &batch(
            "queued_seal",
            vec![entry("new", "wiki/summaries/source/L1/new.md")],
        ),
    )
    .unwrap();

    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let tree = head.tree().unwrap();
    assert!(tree
        .get_path(Path::new("summaries/source/L1/new.md"))
        .is_ok());
    assert!(tree.get_path(Path::new("raw/old.md")).is_err());
}

#[test]
fn commit_summary_opens_only_the_nested_wiki_repo() {
    let dir = TempDir::new().unwrap();
    let wiki = dir.path().join("wiki");
    let summary = wiki.join("summaries/source/L1/summary-1.md");
    std::fs::create_dir_all(summary.parent().unwrap()).unwrap();
    std::fs::write(&summary, "summary").unwrap();

    let parent_repo = Repository::init(dir.path()).unwrap();

    commit_summaries(
        dir.path(),
        &batch(
            "queued_seal",
            vec![entry("summary-1", "wiki/summaries/source/L1/summary-1.md")],
        ),
    )
    .unwrap();

    let repo = Repository::open(&wiki).unwrap();
    let tree = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .tree()
        .unwrap();
    assert!(tree
        .get_path(Path::new("summaries/source/L1/summary-1.md"))
        .is_ok());
    assert!(
        parent_repo.head().is_err(),
        "summary history should not mutate the parent repo"
    );
}

#[test]
fn commit_summary_drops_deleted_summary_entries_from_the_index() {
    let dir = TempDir::new().unwrap();
    let wiki = dir.path().join("wiki");
    let old_summary = wiki.join("summaries/source/L1/old.md");
    let new_summary = wiki.join("summaries/source/L1/new.md");
    std::fs::create_dir_all(old_summary.parent().unwrap()).unwrap();
    std::fs::write(&old_summary, "old summary").unwrap();

    commit_summaries(
        dir.path(),
        &batch(
            "queued_seal",
            vec![entry("old", "wiki/summaries/source/L1/old.md")],
        ),
    )
    .unwrap();

    std::fs::remove_file(&old_summary).unwrap();
    std::fs::write(&new_summary, "new summary").unwrap();
    commit_summaries(
        dir.path(),
        &batch(
            "queued_seal",
            vec![entry("new", "wiki/summaries/source/L1/new.md")],
        ),
    )
    .unwrap();

    let repo = Repository::open(&wiki).unwrap();
    let tree = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .tree()
        .unwrap();
    assert!(tree
        .get_path(Path::new("summaries/source/L1/new.md"))
        .is_ok());
    assert!(tree
        .get_path(Path::new("summaries/source/L1/old.md"))
        .is_err());
}

#[test]
fn commit_summary_recovers_existing_uncommitted_summary_files() {
    let dir = TempDir::new().unwrap();
    let wiki = dir.path().join("wiki");
    let missed_summary = wiki.join("summaries/source/L1/missed.md");
    let new_summary = wiki.join("summaries/source/L1/new.md");
    std::fs::create_dir_all(missed_summary.parent().unwrap()).unwrap();
    std::fs::write(&missed_summary, "missed summary").unwrap();
    std::fs::write(&new_summary, "new summary").unwrap();

    commit_summaries(
        dir.path(),
        &batch(
            "queued_seal",
            vec![entry("new", "wiki/summaries/source/L1/new.md")],
        ),
    )
    .unwrap();

    let repo = Repository::open(&wiki).unwrap();
    let tree = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .tree()
        .unwrap();
    assert!(tree
        .get_path(Path::new("summaries/source/L1/new.md"))
        .is_ok());
    assert!(tree
        .get_path(Path::new("summaries/source/L1/missed.md"))
        .is_ok());
}

#[test]
fn commit_summary_rejects_non_summary_paths() {
    let dir = TempDir::new().unwrap();
    let err = commit_summaries(
        dir.path(),
        &batch("bad", vec![entry("bad", "wiki/notes/one.md")]),
    )
    .unwrap_err();
    assert!(err.to_string().contains("only tracks summary nodes"));
}

#[test]
fn commit_message_describes_seal_metadata() {
    let dir = TempDir::new().unwrap();
    let wiki = dir.path().join("wiki");
    let summary = wiki.join("summaries/source/L2/summary-2.md");
    std::fs::create_dir_all(summary.parent().unwrap()).unwrap();
    std::fs::write(&summary, "---\nkind: summary\n---\nbody").unwrap();

    commit_summaries(
        dir.path(),
        &batch(
            "sync_cascade",
            vec![SummaryCommitEntry {
                summary_id: "summary-2".to_string(),
                content_path: "wiki/summaries/source/L2/summary-2.md".to_string(),
                level: 2,
                child_count: 7,
                token_count: 123,
                time_range_start: ts(1_700_000_000_000),
                time_range_end: ts(1_700_003_600_000),
            }],
        ),
    )
    .unwrap();

    let repo = Repository::open(&wiki).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let msg = head.message().unwrap();
    assert!(msg.contains("Seal memory tree slack:#eng L2 summaries"));
    assert!(msg.contains("Reason: sync_cascade"));
    assert!(msg.contains("Summary-Count: 1"));
    assert!(msg.contains("Child-Count: 7"));
    assert!(msg.contains("Token-Count: 123"));
    assert!(msg.contains("summary-2 L2 children=7 tokens=123"));
}

#[test]
fn read_pointer_tags_are_timestamped_and_move_latest_without_new_commit() {
    let dir = TempDir::new().unwrap();
    let wiki = dir.path().join("wiki");
    let summary = wiki.join("summaries/source/L1/summary-1.md");
    std::fs::create_dir_all(summary.parent().unwrap()).unwrap();
    std::fs::write(&summary, "---\nkind: summary\n---\nbody").unwrap();
    commit_summaries(
        dir.path(),
        &batch(
            "queued_seal",
            vec![entry("summary-1", "wiki/summaries/source/L1/summary-1.md")],
        ),
    )
    .unwrap();

    let repo = Repository::open(&wiki).unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    let head_id = head.id().to_string();

    let tagged = set_read_pointer_tag(dir.path(), "agent:default", None).unwrap();
    assert_eq!(tagged, head_id);
    assert_eq!(
        get_read_pointer_tag(dir.path(), "agent:default")
            .unwrap()
            .as_deref(),
        Some(head_id.as_str())
    );
    let tag_prefix = format!(
        "refs/tags/read/{}/",
        hex::encode("agent:default".as_bytes())
    );
    let tags = repo.references().unwrap().fold(Vec::new(), |mut acc, r| {
        let r = r.unwrap();
        let name = r.name().unwrap();
        if name.starts_with(&tag_prefix) {
            acc.push(name.to_string());
        }
        acc
    });
    assert!(
        tags.iter().any(|name| name.ends_with("/latest")),
        "latest read pointer tag should be present: {tags:?}"
    );
    assert!(
        tags.iter().any(|name| {
            let suffix = name.strip_prefix(&tag_prefix).unwrap_or_default();
            suffix.len() == "20260626T045537.123456789Z".len()
                && suffix.ends_with('Z')
                && suffix.contains('T')
        }),
        "timestamped read pointer tag should be present: {tags:?}"
    );
    let mut walk = repo.revwalk().unwrap();
    walk.push_head().unwrap();
    assert_eq!(
        walk.count(),
        1,
        "moving the read pointer must not create commits"
    );
}

fn batch(reason: &str, entries: Vec<SummaryCommitEntry>) -> SummaryCommitBatch {
    SummaryCommitBatch {
        reason: reason.to_string(),
        tree_id: "tree-1".to_string(),
        tree_scope: "slack:#eng".to_string(),
        entries,
    }
}

fn entry(summary_id: &str, content_path: &str) -> SummaryCommitEntry {
    SummaryCommitEntry {
        summary_id: summary_id.to_string(),
        content_path: content_path.to_string(),
        level: 1,
        child_count: 2,
        token_count: 10,
        time_range_start: ts(1_700_000_000_000),
        time_range_end: ts(1_700_000_001_000),
    }
}

fn ts(ms: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(ms).unwrap()
}
