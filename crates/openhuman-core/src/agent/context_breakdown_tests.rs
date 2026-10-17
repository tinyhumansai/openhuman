use super::*;
use crate::agent::debug::prompt_size::PromptSizeReport;
use crate::agent::debug::DumpedPrompt;

fn sample_dump() -> DumpedPrompt {
    DumpedPrompt {
        agent_id: "orchestrator".into(),
        mode: "session",
        model: "gpt-4o-mini".into(),
        workspace_dir: std::path::PathBuf::from("/tmp"),
        text: "preamble\n# Identity\nyou are an agent\n## Tools\nuse them wisely\n".into(),
        tool_names: vec!["read_file".into(), "write_file".into()],
        skill_tool_count: 0,
        tool_specs: vec![
            serde_json::json!({"name": "read_file", "description": "reads a file", "parameters": {"type": "object"}}),
            serde_json::json!({"name": "write_file", "description": "writes a much longer file with a lot of description text here", "parameters": {"type": "object", "properties": {"path": {"type": "string"}}}}),
        ],
    }
}

#[test]
fn section_from_prompt_carries_heading_and_estimated_tokens() {
    let report = PromptSizeReport::from_dump(&sample_dump());
    let sections: Vec<ContextSection> = report.sections.iter().map(section_from_prompt).collect();
    assert!(!sections.is_empty());
    let total_bytes: usize = report.sections.iter().map(|s| s.bytes).sum();
    let total_from_helper: usize = sections.iter().map(|s| s.bytes).sum();
    assert_eq!(total_bytes, total_from_helper);
    for section in &sections {
        assert_eq!(section.est_tokens, est_tokens(section.bytes));
    }
}

#[test]
fn tools_section_rolls_up_every_tool_into_one_row() {
    let report = PromptSizeReport::from_dump(&sample_dump());
    let section = tools_section(&report.tools);
    assert_eq!(section.label, "tools");
    assert_eq!(section.bytes, report.tool_bytes);
    assert_eq!(section.est_tokens, est_tokens(report.tool_bytes));
}

#[test]
fn est_tokens_divides_by_the_shared_bytes_per_token_constant() {
    assert_eq!(
        est_tokens(400),
        400 / crate::agent::debug::prompt_size::EST_BYTES_PER_TOKEN
    );
    assert_eq!(est_tokens(0), 0);
}

#[test]
fn history_section_excludes_prompt_and_tool_tokens_already_listed() {
    assert_eq!(history_tokens(100_000, 30_000), 70_000);
    assert_eq!(history_tokens(20_000, 30_000), 0);
}

#[test]
fn config_fingerprint_changes_when_config_content_changes() {
    // `workspace_dir` is `#[serde(skip)]` on `Config` (a runtime path, not
    // serialized content), so the fingerprint — deliberately built from the
    // serialized form — must not react to it. Mutate an actually-serialized
    // field instead.
    let mut a = Config::default();
    a.default_temperature = 0.2;
    let mut b = Config::default();
    b.default_temperature = 0.9;
    assert_ne!(config_fingerprint(&a), config_fingerprint(&b));
}

#[test]
fn config_fingerprint_ignores_workspace_dir() {
    let mut a = Config::default();
    a.workspace_dir = std::path::PathBuf::from("/tmp/a");
    let mut b = Config::default();
    b.workspace_dir = std::path::PathBuf::from("/tmp/b");
    assert_eq!(config_fingerprint(&a), config_fingerprint(&b));
}

#[test]
fn config_fingerprint_is_stable_for_identical_config() {
    let a = Config::default();
    let b = Config::default();
    assert_eq!(config_fingerprint(&a), config_fingerprint(&b));
}

#[tokio::test]
async fn history_section_is_none_for_an_unknown_thread() {
    // No transcripts exist for this id, so `token_usage` should report
    // `has_usage: false` (or the RPC itself fails) and this must degrade to
    // `None` rather than propagating an error — the breakdown must still
    // answer with just the prompt/tools sections.
    let ctx = DispatchCtx::process_default();
    let section = history_section(&ctx, "context-breakdown-unknown-thread-id", 0).await;
    assert!(section.is_none());
}

#[test]
fn context_breakdown_params_default_agent_id_is_none() {
    let params = ContextBreakdownParams::default();
    assert!(params.agent_id.is_none());
    assert!(params.thread_id.is_none());
}
