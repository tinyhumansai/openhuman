
/// Heuristic for transient cloud failures the provider stack didn't
/// already classify — connection resets, timeouts, generic 5xx text.
/// Mirrors the conservative match shape used by `is_upstream_unhealthy`.
fn is_transient_string(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    let hints = [
        "timed out",
        "timeout",
        "connection",
        "connect error",
        "broken pipe",
        "reset by peer",
        "deadline exceeded",
        "temporarily unavailable",
    ];
    if hints.iter().any(|h| lower.contains(h)) {
        return true;
    }
    // Bare 5xx in the message body. Be careful not to match arbitrary
    // numerals — only treat 5xx as transient.
    for token in lower.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(code) = token.parse::<u16>() {
            if (500..600).contains(&code) {
                return true;
            }
        }
    }
    false
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn extract_inline_prompt(def: &AgentDefinition) -> Option<String> {
    match &def.system_prompt {
        PromptSource::Inline(body) if !body.is_empty() => Some(body.clone()),
        PromptSource::Dynamic(build) => {
            use crate::openhuman::agent::context::prompt::{
                ConnectedIntegration, LearnedContextData, PromptContext, PromptTool, ToolCallFormat,
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
                learned: LearnedContextData::default(),
                visible_tool_names: &empty_visible,
                tool_call_format: ToolCallFormat::PFormat,
                connected_integrations: &empty_integrations,
                connected_identities_md: String::new(),
                include_profile: false,
                include_memory_md: false,
                curated_snapshot: None,
                user_identity: None,
                personality_soul_md: None,
                personality_memory_md: None,
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

fn render_user_message(envelope: &TriggerEnvelope) -> String {
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

fn format_parse_error(err: &ParseError) -> String {
    match err {
        ParseError::NoJsonObject => "classifier reply had no JSON object".to_string(),
        ParseError::InvalidJson(src) => format!("classifier JSON invalid: {src}"),
        ParseError::MissingTarget { action } => {
            format!("action `{action}` missing required target_agent/prompt")
        }
    }
}

fn truncate_payload(payload: &serde_json::Value, max_bytes: usize) -> String {
    let pretty = serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string());
    if pretty.len() <= max_bytes {
        return pretty;
    }
    let dropped = pretty.len() - max_bytes;
    let end = crate::openhuman::util::floor_char_boundary(&pretty, max_bytes);
    format!("{}\n[...truncated {dropped} bytes]", &pretty[..end])
}
