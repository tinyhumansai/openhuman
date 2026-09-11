//! `write_pause_checkpoint` — the pause-persistence contract (#5928).
//!
//! Every failure branch returns `None`, and that `None` is what stops the
//! runner reporting a resumable `AwaitingUser` for a pause it never saved.
//! Before this, all three failures were logged at `warn` and then dropped, so
//! the orchestrator relayed a `task_id` and asked the user to answer a question
//! whose answer had nowhere to go.

use crate::openhuman::agent::harness::subagent_runner::types::SubagentCheckpointData;

fn checkpoint_data(task_id: &str) -> SubagentCheckpointData {
    SubagentCheckpointData {
        task_id: task_id.to_string(),
        agent_id: "researcher".to_string(),
        worker_thread_id: None,
        history: Vec::new(),
        question: "Which region?".to_string(),
        options: None,
        toolkit_override: None,
        skill_filter_override: None,
        model_override: None,
        created_at: "2026-09-02T00:00:00Z".to_string(),
    }
}

#[test]
fn a_written_checkpoint_returns_the_path_it_wrote() {
    let dir = tempfile::tempdir().expect("tempdir");
    let checkpoint_dir = dir.path().join("subagent_checkpoints");

    let written = super::write(&checkpoint_dir, "task-1", &checkpoint_data("task-1"))
        .expect("a writable directory must produce a checkpoint");

    assert_eq!(
        written,
        checkpoint_dir.join("task-1.json"),
        "the returned path must be the one the resume flow will read"
    );
    assert!(written.is_file(), "the checkpoint must exist on disk");

    // Round-trips: a resume reads this back with `serde_json::from_str`, so a
    // path that exists but cannot be parsed is no better than a missing one.
    let raw = std::fs::read_to_string(&written).expect("read back");
    let parsed: SubagentCheckpointData = serde_json::from_str(&raw).expect("checkpoint parses");
    assert_eq!(parsed.task_id, "task-1");
    assert_eq!(parsed.question, "Which region?");
}

#[test]
fn the_directory_is_created_on_demand() {
    // #5928 asks whether the checkpoint directory is created correctly on fresh
    // installs. It is: nothing pre-creates it, `write_pause_checkpoint` does,
    // including intermediate components.
    let dir = tempfile::tempdir().expect("tempdir");
    let nested = dir.path().join("a/b/c/subagent_checkpoints");
    assert!(!nested.exists(), "precondition: nothing has created it");

    let written = super::write(&nested, "task-2", &checkpoint_data("task-2"))
        .expect("a missing directory must be created, not treated as a failure");

    assert!(written.is_file());
}

#[test]
fn an_uncreatable_directory_reports_no_checkpoint() {
    // A regular file where the directory should be: `create_dir_all` cannot
    // succeed, and the pause is not resumable from disk.
    let dir = tempfile::tempdir().expect("tempdir");
    let blocked = dir.path().join("not_a_dir");
    std::fs::write(&blocked, b"i am a file").expect("seed the blocker");

    assert!(
        super::write(&blocked, "task-3", &checkpoint_data("task-3")).is_none(),
        "a checkpoint directory that cannot exist must report no checkpoint, \
         not a silently dropped warning"
    );
}

#[test]
fn an_unwritable_target_reports_no_checkpoint() {
    // The directory exists, but the checkpoint's own path is occupied by a
    // directory, so the write cannot land.
    let dir = tempfile::tempdir().expect("tempdir");
    let checkpoint_dir = dir.path().join("subagent_checkpoints");
    std::fs::create_dir_all(checkpoint_dir.join("task-4.json")).expect("occupy the target path");

    assert!(
        super::write(&checkpoint_dir, "task-4", &checkpoint_data("task-4")).is_none(),
        "a checkpoint whose write fails must report no checkpoint"
    );
}

// ── task_id is untrusted on its way into a path (tinysweeper, #5951) ────────
//
// `continue_subagent` takes `task_id` from its tool arguments — model-authored
// — and feeds it back through `SubagentRunOptions::task_id`, so a re-paused
// child writes a checkpoint under a name the model chose. Joined unchecked,
// `../` walks the write out of the checkpoint directory entirely.

#[test]
fn a_traversing_task_id_is_refused_rather_than_written() {
    let dir = tempfile::tempdir().expect("tempdir");
    let checkpoint_dir = dir.path().join("subagent_checkpoints");
    let outside = dir.path().join("outside.json");

    assert!(
        super::write(
            &checkpoint_dir,
            "../outside",
            &checkpoint_data("../outside")
        )
        .is_none(),
        "a traversing task id must not produce a checkpoint"
    );
    assert!(
        !outside.exists(),
        "the write must not land outside the checkpoint directory"
    );

    for bad in [
        "../../etc/cron.d/pwn",
        "a/b",
        "a\\b",
        "..",
        ".",
        "",
        "with space",
        "sub-\u{00e9}",
    ] {
        assert!(
            !super::is_safe_task_id(bad),
            "{bad:?} must be rejected as a checkpoint filename"
        );
    }
}

#[test]
fn the_ids_this_system_actually_mints_are_accepted() {
    // The guard is an allow-list, so it has to admit every shape in real use —
    // otherwise it would break resumption instead of hardening it.
    for good in [
        "sub-2b9d1f4e-0c3a-4f10-9c2e-6a7b8c9d0e1f",
        "subsess-2b9d1f4e-0c3a-4f10-9c2e-6a7b8c9d0e1f",
        "task-fleet-b",
        "t1",
        "t_steer",
    ] {
        assert!(
            super::is_safe_task_id(good),
            "{good:?} is a real id shape and must be accepted"
        );
    }
}
