//! Prompt assembly for the classifier turn: the system prompt from the
//! built-in agent definition and the rendered user message.

use crate::agent::harness::definition::{AgentDefinition, PromptSource};

use super::super::decision::ParseError;
use super::super::envelope::TriggerEnvelope;

/// How much of the raw payload we inline into the user message.
const PAYLOAD_INLINE_LIMIT_BYTES: usize = 8 * 1024;

pub(crate) fn extract_inline_prompt(def: &AgentDefinition) -> Option<String> {
    match &def.system_prompt {
        PromptSource::Inline(body) if !body.is_empty() => Some(body.clone()),
        PromptSource::Dynamic(build) => {
            use crate::agent::prompts::{
                ConnectedIntegration, PromptContext, PromptTool, ToolCallFormat,
            };
            let empty_tools: Vec<PromptTool<'_>> = Vec::new();
            let empty_integrations: Vec<ConnectedIntegration> = Vec::new();
            let empty_visible: std::collections::HashSet<String> = std::collections::HashSet::new();
            let ctx = PromptContext {
                workspace_dir: std::path::Path::new("."),
                model_name: "",
                agent_id: &def.id,
                tools: &empty_tools,
                workflows: &[],
                dispatcher_instructions: "",
                visible_tool_names: &empty_visible,
                tool_call_format: ToolCallFormat::PFormat,
                connected_integrations: &empty_integrations,
                connected_identities_md: String::new(),
                user_identity: None,
                personality_roster: vec![],
                agents_md_global: None,
                agents_md_local: None,
            };
            match build(&ctx) {
                Ok(body) if !body.is_empty() => Some(body),
                Ok(_) => None,
                Err(e) => {
                    tracing::warn!(
                        agent_id = %def.id,
                        error = %e,
                        "[triage::evaluator] dynamic prompt builder failed"
                    );
                    None
                }
            }
        }
        _ => None,
    }
}

pub(crate) fn render_user_message(envelope: &TriggerEnvelope) -> String {
    let payload_string = truncate_payload(&envelope.payload, PAYLOAD_INLINE_LIMIT_BYTES);
    format!(
        "SOURCE: {source}\n\
         DISPLAY_LABEL: {label}\n\
         EXTERNAL_ID: {eid}\n\
         PAYLOAD:\n{payload}",
        source = envelope.source.slug(),
        label = envelope.display_label,
        eid = envelope.external_id,
        payload = payload_string,
    )
}

pub(super) fn format_parse_error(err: &ParseError) -> String {
    match err {
        ParseError::NoJsonObject => "classifier reply had no JSON object".to_string(),
        ParseError::InvalidJson(src) => format!("classifier JSON invalid: {src}"),
        ParseError::MissingTarget { action } => {
            format!("action `{action}` missing required target_agent/prompt")
        }
    }
}

pub(crate) fn truncate_payload(payload: &serde_json::Value, max_bytes: usize) -> String {
    let pretty = serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());
    if pretty.len() <= max_bytes {
        return pretty;
    }
    let dropped = pretty.len() - max_bytes;
    let end = crate::util::floor_char_boundary(&pretty, max_bytes);
    format!("{}\n[...truncated {dropped} bytes]", &pretty[..end])
}
