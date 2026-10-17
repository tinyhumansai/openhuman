//! The pause task-id guard and the checkpoint shape (#5928).

use crate::agent::subagent_host::types::SubagentCheckpointData;

fn checkpoint_data(task_id: &str) -> SubagentCheckpointData {
    SubagentCheckpointData {
        task_key: None,
        task_id: task_id.to_string(),
        agent_id: "researcher".to_string(),
        worker_thread_id: None,
        history: Vec::new(),
        question: "Which region?".to_string(),
        options: None,
        skill_filter_override: None,
        model_override: None,
        created_at: "2026-09-02T00:00:00Z".to_string(),
        resume_checkpoint: None,
        resume_metadata: Default::default(),
        output: String::new(),
        iterations: 0,
        usage: Default::default(),
        artifact_paths: Vec::new(),
        elapsed_ms: 0,
    }
}

// ── task_id is untrusted on its way into a path (tinysweeper, #5951) ────────
//
// `continue_subagent` takes `task_id` from its tool arguments — model-authored
// — and feeds it back through `SubagentRunOptions::task_id`, so a re-paused
// child writes a checkpoint under a name the model chose. Joined unchecked,
// `../` walks the write out of the checkpoint directory entirely.

#[test]
fn a_traversing_task_id_is_refused() {
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

#[test]
fn a_legacy_checkpoint_with_toolkit_override_still_parses() {
    // Checkpoints written before the `toolkit` spawn argument was retired
    // carry a `toolkit_override` key. Resume must still read them.
    let mut value = serde_json::to_value(checkpoint_data("task-legacy")).expect("serialise");
    value
        .as_object_mut()
        .expect("checkpoint is an object")
        .insert("toolkit_override".into(), serde_json::json!("gmail"));
    let parsed: SubagentCheckpointData =
        serde_json::from_value(value).expect("legacy checkpoint parses");
    assert_eq!(parsed.task_id, "task-legacy");
    assert_eq!(parsed.question, "Which region?");
}
