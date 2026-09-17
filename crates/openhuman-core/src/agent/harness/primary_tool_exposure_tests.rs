use super::*;
use serde_json::json;

fn spec(name: &str, description: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: description.to_string(),
        parameters: json!({"type": "object"}),
    }
}

fn catalogue() -> Vec<ToolSpec> {
    vec![
        spec("ask_user_clarification", "Ask the user a question."),
        spec("use_skill", "Load a skill."),
        spec("tool_search", "Find a deferred tool."),
        spec("web_fetch", "Fetch content from a web address."),
        spec("browser_open", "Open a website in the browser."),
        spec("file_read", "Read a workspace file."),
        spec("file_write", "Write a workspace file."),
        spec("run_tests", "Run repository tests."),
        spec("memory_recall", "Recall stored memories."),
        spec("image_generate", "Generate an image."),
    ]
}

#[test]
fn conversational_prompt_exposes_only_recovery_tools() {
    let selected = plan_primary_tool_exposure("hey", &catalogue(), None, &HashSet::new());
    assert_eq!(selected.len(), RECOVERY_TOOLS.len());
    assert!(RECOVERY_TOOLS.iter().all(|name| selected.contains(*name)));
}

#[test]
fn category_keywords_do_not_match_inside_unrelated_words() {
    let selected =
        plan_primary_tool_exposure("show my profile", &catalogue(), None, &HashSet::new());
    assert!(!selected.contains("file_read"));
    assert!(!selected.contains("file_write"));
}

#[test]
fn proper_noun_news_prompt_gets_web_family_not_workspace_tools() {
    let selected = plan_primary_tool_exposure(
        "show me the top three Google News headlines",
        &catalogue(),
        None,
        &HashSet::new(),
    );
    assert!(selected.contains("web_fetch"));
    assert!(selected.contains("browser_open"));
    assert!(!selected.contains("file_read"));
    assert!(!selected.contains("image_generate"));
}

#[test]
fn image_retrieval_does_not_expose_generation_tools() {
    let contract = plan_primary_turn_contract(
        "get me a pic from the internet; don't generate a pic",
        &catalogue(),
        None,
        &HashSet::new(),
    );
    assert_eq!(contract.intent_family, PrimaryIntentFamily::ImageRetrieval);
    assert!(contract.allowed_tools.contains("web_fetch"));
    assert!(contract.allowed_tools.contains("browser_open"));
    assert!(!contract.allowed_tools.contains("image_generate"));
    assert!(!contract.allowed_tools.contains("use_skill"));
}

#[test]
fn image_generation_does_not_implicitly_expose_web_tools() {
    let contract =
        plan_primary_turn_contract("generate a portrait", &catalogue(), None, &HashSet::new());
    assert_eq!(contract.intent_family, PrimaryIntentFamily::ImageGeneration);
    assert!(contract.allowed_tools.contains("image_generate"));
    assert!(!contract.allowed_tools.contains("web_fetch"));
    assert!(!contract.allowed_tools.contains("browser_open"));
}

#[test]
fn registered_memory_recall_stays_in_prompt_execution_contract() {
    let selected = plan_primary_tool_exposure("hey", &catalogue(), None, &HashSet::new());
    assert!(selected.contains("memory_recall"));
}

#[test]
fn ceiling_is_fail_closed_and_state_cannot_widen_it() {
    let ceiling = HashSet::from(["web_fetch".to_string(), "use_skill".to_string()]);
    let state = HashSet::from(["file_write".to_string()]);
    let selected = plan_primary_tool_exposure(
        "write the file and fetch news",
        &catalogue(),
        Some(&ceiling),
        &state,
    );
    assert_eq!(selected, ceiling);

    let denied = plan_primary_tool_exposure(
        "fetch news",
        &catalogue(),
        Some(&HashSet::new()),
        &HashSet::new(),
    );
    assert!(denied.is_empty());
}

#[test]
fn ordinary_tools_are_capped() {
    let mut candidates = catalogue();
    for i in 0..20 {
        candidates.push(spec(&format!("web_action_{i}"), "Search news on the web."));
    }
    let selected = plan_primary_tool_exposure(
        "search the web for news",
        &candidates,
        None,
        &HashSet::new(),
    );
    let ordinary = selected
        .iter()
        .filter(|name| !RECOVERY_TOOLS.contains(&name.as_str()))
        .count();
    assert_eq!(ordinary, MAX_PROMPT_SELECTED_TOOLS);
}

#[test]
fn representative_news_turn_reduces_model_visible_schema_bytes() {
    let mut candidates = catalogue();
    for i in 0..20 {
        candidates.push(spec(
            &format!("unrelated_workspace_action_{i}"),
            "Inspect or modify unrelated workspace state with a verbose argument contract.",
        ));
    }
    let selected = plan_primary_tool_exposure(
        "show me the top three Google News headlines",
        &candidates,
        None,
        &HashSet::new(),
    );
    let full_bytes: usize = candidates
        .iter()
        .map(|spec| serde_json::to_vec(spec).unwrap().len())
        .sum();
    let selected_bytes: usize = candidates
        .iter()
        .filter(|spec| selected.contains(&spec.name))
        .map(|spec| serde_json::to_vec(spec).unwrap().len())
        .sum();

    assert!(
        selected_bytes < full_bytes / 2,
        "{selected_bytes} vs {full_bytes}"
    );
}
