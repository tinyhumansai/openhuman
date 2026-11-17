use super::*;

#[test]
fn prompt_framing_matches_the_child_spawn_surface() {
    let nested = ["spawn_subagent".to_owned()].into_iter().collect();
    let nested_prompt = compose_subagent_system_prompt(
        "Worker instructions".into(),
        "worker",
        &nested,
        "complete the task",
        1,
    );
    assert!(nested_prompt.contains("You may delegate parts of the work"));
    assert!(!nested_prompt.contains("You are a leaf worker"));

    let leaf_prompt = compose_subagent_system_prompt(
        "Worker instructions".into(),
        "worker",
        &HashSet::new(),
        "complete the task",
        1,
    );
    assert!(leaf_prompt.contains("You are a leaf worker"));
}
