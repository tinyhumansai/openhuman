use super::*;

use tinyagents_harness_tools::scratchpad::TaskNotes;
use tinyagents_harness_tools::todos::{TodoItem, TodoStatus};

fn plan(first_step: &str) -> ClosedPlan {
    ClosedPlan {
        thread_id: "t".to_string(),
        items: vec![TodoItem::with_status(first_step, TodoStatus::Completed)],
        notes: TaskNotes {
            facts: vec!["the table is auth_users".to_string()],
            blockers: vec!["ALTER without a default timed out".to_string()],
        },
    }
}

#[test]
fn facts_become_fact_learnings_and_blockers_corrections() {
    assert_eq!(
        learnings(&plan("Dump the schema")),
        vec![
            ("the table is auth_users".to_string(), "fact"),
            (
                "Blocker: ALTER without a default timed out".to_string(),
                "correction"
            ),
        ]
    );
}

#[test]
fn the_task_is_the_goal_else_the_first_step_on_one_bounded_line() {
    assert_eq!(
        task_label(&plan("Dump"), Some("Add org_id\nwithout downtime")),
        "Add org_id without downtime"
    );
    assert_eq!(
        task_label(&plan("Dump the schema"), None),
        "Dump the schema"
    );
    let long = "x".repeat(MAX_TASK_CHARS + 10);
    let label = task_label(&plan(&long), None);
    assert_eq!(label.chars().count(), MAX_TASK_CHARS);
    assert!(label.ends_with('…'));
}
