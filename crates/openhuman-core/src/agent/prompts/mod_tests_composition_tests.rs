use super::*;
use tinyagents_harness::prompt::{
    PromptTier, PROMPT_TIER_CONTEXT_MARKER, PROMPT_TIER_VOLATILE_MARKER,
};

#[test]
fn composed_prompt_keeps_context_and_volatile_system_message_boundaries() {
    static RENDERS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    fn dynamic_prompt(_ctx: &PromptContext<'_>) -> anyhow::Result<String> {
        RENDERS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(format!(
            "## Grounding and tool use\n\nProduct grounding.\nStable 📍\n{context}\n## Project context\nContext instructions 🧭.\n{volatile}\nVolatile identity.",
            context = PROMPT_TIER_CONTEXT_MARKER,
            volatile = PROMPT_TIER_VOLATILE_MARKER,
        ))
    }

    let workspace = tempfile::tempdir().unwrap();
    let custom_style = "# User style\nKeep this exact custom style.";
    std::fs::write(workspace.path().join("STYLE.md"), custom_style).unwrap();
    let ctx = PromptContext {
        workspace_dir: workspace.path(),
        model_name: "test-model",
        agent_id: "",
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
        flow_archetype_text: None,
    };

    RENDERS.store(0, std::sync::atomic::Ordering::SeqCst);
    let rendered = SystemPromptBuilder::from_dynamic(dynamic_prompt)
        .build_tiered(&ctx)
        .unwrap();
    assert_eq!(RENDERS.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(!rendered.breakpoints.is_empty());
    assert!(rendered
        .breakpoints
        .iter()
        .all(|offset| rendered.text.is_char_boundary(*offset)));
    let stable_boundary = rendered.breakpoints[0];
    assert!(rendered.text[..stable_boundary].contains("Stable 📍"));
    assert!(rendered.text[..stable_boundary].contains(custom_style));
    assert!(!rendered.text[..stable_boundary].contains("Context instructions 🧭."));
    let messages = rendered.system_messages();
    assert_eq!(
        messages.len(),
        2,
        "Stable+Context remain first, Volatile second"
    );
    assert!(messages[0].contains("Product grounding."));
    assert!(messages[0].contains("Context instructions 🧭."));
    assert!(messages[0].contains("Stable 📍"));
    assert!(messages[0].contains(custom_style));
    assert!(!messages[0].contains("Volatile identity."));
    assert_eq!(messages[0].matches("## Grounding and tool use").count(), 1);
    assert!(messages[1].contains("Volatile identity."));
    assert!(!messages[1].contains(custom_style));
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("STYLE.md")).unwrap(),
        custom_style,
        "composition must preserve the user-edited STYLE.md bytes"
    );
}

#[test]
fn unmarked_dynamic_body_stays_volatile_after_stable_and_context_messages() {
    fn dynamic_prompt(_ctx: &PromptContext<'_>) -> anyhow::Result<String> {
        Ok("Unmarked dynamic UTF-8 📎 body.".to_string())
    }
    let workspace = tempfile::tempdir().unwrap();
    let ctx = PromptContext {
        workspace_dir: workspace.path(),
        model_name: "test-model",
        agent_id: "",
        tools: &[],
        workflows: &[],
        dispatcher_instructions: "",
        visible_tool_names: &NO_FILTER,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        user_identity: None,
        personality_roster: vec![],
        agents_md_global: None,
        agents_md_local: None,
        flow_archetype_text: None,
    };
    let rendered = SystemPromptBuilder::from_dynamic(dynamic_prompt)
        .build_tiered(&ctx)
        .unwrap();
    let messages = rendered.system_messages();
    assert_eq!(
        messages.len(),
        2,
        "the stable prefix and unmarked volatile body stay separate"
    );
    assert!(rendered.parts.iter().any(|(tier, part)| {
        *tier == PromptTier::Volatile && part.contains("Unmarked dynamic UTF-8 📎 body.")
    }));
    assert!(messages[0].contains("## Grounding and tool use"));
    assert!(messages[1].contains("Unmarked dynamic UTF-8 📎 body."));
}

#[test]
fn empty_verbatim_prompt_keeps_legacy_flattened_newline_without_system_messages() {
    let rendered = SystemPromptBuilder::verbatim(String::new())
        .build_tiered(&ctx_with_identity(None))
        .unwrap();

    assert_eq!(rendered.text, "\n");
    assert!(rendered.parts.is_empty());
    assert!(rendered.system_messages().is_empty());
}
