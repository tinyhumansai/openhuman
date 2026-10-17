use super::*;
use crate::agent::harness::host_definition::AgentDefinitionRegistry;

fn sample_tool() -> ArchetypeDelegationTool {
    ArchetypeDelegationTool {
        tool_name: "delegate_researcher".to_string(),
        agent_id: DelegationTarget("researcher".to_string()),
        tool_description: "Use for web and docs research.".to_string(),
    }
}

#[test]
fn metadata_methods_expose_name_description_and_system_category() {
    let tool = sample_tool();
    assert_eq!(tool.name(), "delegate_researcher");
    assert_eq!(tool.description(), "Use for web and docs research.");
    assert_eq!(tool.permission_level(), PermissionLevel::Execute);
    assert_eq!(tool.category(), ToolCategory::System);
}

#[test]
fn delegation_opts_out_of_the_global_tool_timeout() {
    // A delegated sub-agent run (delegate_tools_agent / run_code / …) can
    // legitimately outlast the single-tool wall-clock default (120s): under
    // `Inherit` every such run is hard-killed and truncated (Sentry
    // TAURI-RUST-K29 / TAURI-RUST-8HB). The child bounds its own lifetime
    // via its max_iterations, the run cancellation token, and each inner
    // tool's own timeout — so this primitive must be Unbounded, like
    // spawn_parallel_agents and the long-running scripting tools.
    assert_eq!(
        sample_tool().timeout_policy(&json!({})),
        ToolTimeout::Unbounded,
    );
}

#[test]
fn parameters_schema_advertises_async_default_blocking_opt_in() {
    // Delegations are async by default (durable worker + follow-up
    // delivery turn); `blocking: true` is the explicit opt-in for
    // results that must gate the current reply. The flag must be
    // advertised but never required.
    let schema = sample_tool().parameters_schema();
    let blocking = &schema["properties"]["blocking"];
    assert_eq!(blocking["type"], "boolean");
    let desc = blocking["description"].as_str().unwrap_or_default();
    assert!(desc.contains("async"), "explains the async default: {desc}");
    assert!(
        desc.contains("Default false"),
        "names which value is the default: {desc}"
    );
    // The resume contract (`subagent_session_id`, `continue_subagent`,
    // `steer_subagent`, …) used to be spelled out here, at 19x the cost.
    // It now lives once in the orchestrator prompt, which
    // `prompt_documents_the_stripped_envelope_fields` pins.
    assert_eq!(schema["required"], json!(["prompt"]));
}

#[test]
fn parameters_schema_requires_prompt_only() {
    let tool = sample_tool();
    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    assert_eq!(schema["required"], json!(["prompt"]));
    assert_eq!(schema["properties"]["prompt"]["type"], "string");

    // Only `prompt`, `blocking`, and explicit `image_paths` are advertised: the structured hand-off
    // fields cost ~150 tokens per delegate on every request, and a
    // self-contained `prompt` carries the same content. They are still
    // parsed (see `structured_handoff_renders_compact_child_prompt`), so a
    // caller that sends them keeps working; they are just not offered.
    let props = schema["properties"]
        .as_object()
        .expect("properties is an object");
    let mut present: Vec<&str> = props.keys().map(String::as_str).collect();
    present.sort_unstable();
    assert_eq!(present, vec!["blocking", "image_paths", "prompt"]);
}

/// Every `description` in the envelope, as `(json-pointer-ish path, text)`.
fn collect_descriptions(node: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match node {
        Value::Object(map) => {
            for (key, value) in map {
                if key == "description" {
                    if let Some(text) = value.as_str() {
                        out.push((path.to_string(), text.to_string()));
                    }
                } else {
                    collect_descriptions(value, &format!("{path}/{key}"), out);
                }
            }
        }
        Value::Array(items) => {
            for (idx, item) in items.iter().enumerate() {
                collect_descriptions(item, &format!("{path}/{idx}"), out);
            }
        }
        _ => {}
    }
}

#[test]
fn envelope_descriptions_stay_within_budget() {
    // This schema is emitted once per synthesised `delegate_*` tool, so prose
    // here is billed per delegate per turn. Fully described it was 356 tokens
    // each; the envelope keeps three short fields. `prompt` (which says the worker has
    // no memory of this chat) and `blocking` (whose default is not in its
    // name), and explicit image forwarding stay under the ~50-token cap.
    let schema = sample_tool().parameters_schema();
    let mut found = Vec::new();
    collect_descriptions(&schema, "", &mut found);

    let mut fields: Vec<&str> = found.iter().map(|(path, _)| path.as_str()).collect();
    fields.sort_unstable();
    assert_eq!(
        fields,
        vec![
            "/properties/blocking",
            "/properties/image_paths",
            "/properties/prompt"
        ],
        "a field or description came back into the delegation envelope; \
         every word here is paid per delegate on every request"
    );

    // ~4 chars per token on this vocabulary, so 220 chars ~= the 50-token cap.
    for (field, text) in &found {
        assert!(
            text.len() <= 220,
            "{field} description is {} chars, over the ~50-token cap: {text}",
            text.len()
        );
    }
    let prompt_desc = schema["properties"]["prompt"]["description"]
        .as_str()
        .unwrap_or_default();
    assert!(
        prompt_desc.contains("no memory of this chat"),
        "the prompt field must say the worker starts cold: {prompt_desc}"
    );
}

#[test]
fn prompt_documents_the_stripped_envelope_fields() {
    // The advertised envelope is `prompt` + `blocking`; what the model must
    // still be taught once, in the parent prompt, is when to block and how to
    // resume a paused worker. The unadvertised hand-off fields must NOT be
    // taught there any more: a prompt naming fields the schema does not offer
    // sends the model arguments it cannot see.
    const ORCHESTRATOR_PROMPT: &str = include_str!("../../registry/agents/orchestrator/prompt.md");

    for needle in ["blocking: true", "continue_subagent"] {
        assert!(
            ORCHESTRATOR_PROMPT.contains(needle),
            "orchestrator/prompt.md no longer teaches `{needle}`"
        );
    }
    for stale in ["must_not_assume", "expected_output", "citation_requirement"] {
        assert!(
            !ORCHESTRATOR_PROMPT.contains(stale),
            "orchestrator/prompt.md still names `{stale}`, which the delegation \
             schema no longer advertises"
        );
    }
}

#[test]
fn structured_handoff_renders_compact_child_prompt() {
    let rendered = render_structured_handoff(
        "Check this",
        &json!({
            "prompt": "Check this",
            "objective": "Answer with supported claims only.",
            "evidence": ["file:src/lib.rs", "tool output: count=3", ""],
            "constraints": ["Do not edit files"],
            "must_not_assume": ["Current service state"],
            "expected_output": "Findings list",
            "citation_requirement": "file_paths",
        }),
    );

    assert!(rendered.contains("Task:\nCheck this"));
    assert!(rendered.contains("Objective:\nAnswer with supported claims only."));
    assert!(rendered.contains("Evidence:\n- file:src/lib.rs\n- tool output: count=3"));
    assert!(rendered.contains("Must not assume:\n- Current service state"));
    assert!(rendered.contains("Citation requirement:\nfile_paths"));
    assert!(!rendered.contains("\"model\""));
}

#[tokio::test]
async fn execute_rejects_missing_or_blank_prompt() {
    let tool = sample_tool();

    let missing = tool.execute(json!({})).await.unwrap();
    assert!(missing.is_error);
    assert!(missing.output().contains("`prompt` is required"));

    let blank = tool.execute(json!({ "prompt": "   " })).await.unwrap();
    assert!(blank.is_error);
    assert!(blank.output().contains("`prompt` is required"));
}

#[tokio::test]
async fn execute_accepts_non_empty_prompt_and_reaches_dispatch_path() {
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let tool = sample_tool();
    let result = tool
        .execute(json!({ "prompt": "find the answer" }))
        .await
        .unwrap();

    let out = result.output();
    assert!(
        !out.contains("`prompt` is required"),
        "non-empty prompt should bypass local validation, got: {out}"
    );
}
