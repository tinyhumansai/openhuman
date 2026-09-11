use super::*;

fn good_def() -> WorkflowDefinition {
    definition_by_id(PARALLEL_RESEARCH_ID).expect("builtin present")
}

#[test]
fn builtin_is_structurally_valid() {
    assert!(validate_structure(&good_def()).is_empty());
}

#[test]
fn builtin_agents_pass_when_all_known() {
    // Treat the four referenced agents as registered.
    let known = ["planner", "researcher", "critic", "summarizer"];
    let errors = validate_agents(&good_def(), |id| known.contains(&id));
    assert!(errors.is_empty(), "unexpected: {errors:?}");
}

#[test]
fn unknown_agent_is_reported() {
    let errors = validate_agents(&good_def(), |id| id == "researcher");
    // planner, critic, summarizer are unknown -> 3 errors.
    assert_eq!(errors.len(), 3);
    assert!(errors.iter().any(
        |e| matches!(e, DefinitionError::UnknownAgent { agent_id, .. } if agent_id == "planner")
    ));
}

#[test]
fn no_phases_is_rejected() {
    let mut def = good_def();
    def.phases.clear();
    assert_eq!(validate_structure(&def), vec![DefinitionError::NoPhases]);
}

#[test]
fn duplicate_and_empty_phase_are_reported() {
    let def = WorkflowDefinition {
        phases: vec![
            WorkflowPhase {
                name: "a".into(),
                description: String::new(),
                agent_ids: vec!["researcher".into()],
                depends_on: vec![],
            },
            WorkflowPhase {
                name: "a".into(),
                description: String::new(),
                agent_ids: vec![],
                depends_on: vec![],
            },
        ],
        ..good_def()
    };
    let errors = validate_structure(&def);
    assert!(errors.contains(&DefinitionError::DuplicatePhase { name: "a".into() }));
    assert!(errors.contains(&DefinitionError::EmptyPhase { phase: "a".into() }));
}

#[test]
fn unknown_dependency_is_reported() {
    let def = WorkflowDefinition {
        phases: vec![WorkflowPhase {
            name: "only".into(),
            description: String::new(),
            agent_ids: vec!["researcher".into()],
            depends_on: vec!["ghost".into()],
        }],
        ..good_def()
    };
    let errors = validate_structure(&def);
    assert!(errors.contains(&DefinitionError::UnknownDependency {
        phase: "only".into(),
        depends_on: "ghost".into(),
    }));
}

#[test]
fn cycle_is_detected() {
    let def = WorkflowDefinition {
        phases: vec![
            WorkflowPhase {
                name: "a".into(),
                description: String::new(),
                agent_ids: vec!["researcher".into()],
                depends_on: vec!["b".into()],
            },
            WorkflowPhase {
                name: "b".into(),
                description: String::new(),
                agent_ids: vec!["researcher".into()],
                depends_on: vec!["a".into()],
            },
        ],
        ..good_def()
    };
    assert!(validate_structure(&def).contains(&DefinitionError::CyclicDependency));
}

#[test]
fn duplicate_phase_names_do_not_report_false_cycle() {
    let def = WorkflowDefinition {
        phases: vec![
            WorkflowPhase {
                name: "a".into(),
                description: String::new(),
                agent_ids: vec!["researcher".into()],
                depends_on: vec![],
            },
            WorkflowPhase {
                name: "a".into(),
                description: String::new(),
                agent_ids: vec!["researcher".into()],
                depends_on: vec![],
            },
        ],
        ..good_def()
    };
    let errors = validate_structure(&def);
    assert!(errors.contains(&DefinitionError::DuplicatePhase { name: "a".into() }));
    assert!(
        !errors.contains(&DefinitionError::CyclicDependency),
        "duplicate names must not trip a false cycle: {errors:?}"
    );
}

#[test]
fn zero_concurrency_is_rejected() {
    let def = WorkflowDefinition {
        default_concurrency: 0,
        max_children: 0,
        ..good_def()
    };
    assert!(
        validate_structure(&def).contains(&DefinitionError::InvalidConcurrency {
            default_concurrency: 0,
            max_children: 0,
        })
    );
}

#[test]
fn list_definitions_returns_builtins() {
    let resp = list_definitions();
    assert_eq!(resp.count, 1);
    assert_eq!(resp.definitions[0].id, PARALLEL_RESEARCH_ID);
}
