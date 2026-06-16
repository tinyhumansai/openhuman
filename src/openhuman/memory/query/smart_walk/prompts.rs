//! Prompt construction, content inventory, model resolution, tool-call parsing,
//! and fallback synthesis for smart_walk.

use crate::openhuman::config::Config;
use crate::openhuman::memory::query::smart_walk::types::{truncate_chars, Evidence, SmartWalkStep};
use std::path::Path;

// ── Inner call type (used by parser and dispatch) ───────────────────────────

#[derive(Clone)]
pub(crate) struct InnerCall {
    pub(crate) name: String,
    pub(crate) args: serde_json::Value,
}

// ── System prompt ────────────────────────────────────────────────────────────

pub(crate) fn build_system_prompt() -> String {
    r#"You are a smart memory retrieval agent. Your task is to answer queries by
searching through a user's personal memory — which includes raw files (emails,
chats, commits, documents), wiki summaries, episodic conversation memories,
and document archives.

## Strategy

Use a multi-strategy approach inspired by graph-based retrieval:

1. **Start broad**: Use `list_sources` to understand what content is available,
   then `keyword_search` or `vector_search` to find relevant starting points.

2. **Follow connections**: When you find a relevant entity or topic, use
   `entity_search` to find related entities and follow the connections.

3. **Drill into details**: Use `read_content` to read specific files for
   full context. Use `browse_tree` to navigate wiki summary hierarchies.

4. **Collect evidence**: As you find relevant information, use `collect_evidence`
   to save snippets. This builds your citation buffer for the final answer.

5. **Synthesize**: When you have enough evidence, use `answer` to provide a
   comprehensive response with citations.

## Rules

- Be efficient: don't re-search for things you already found.
- Prefer vector_search for semantic/conceptual queries.
- Prefer keyword_search for specific names, IDs, or exact phrases.
- Use entity_search when the query mentions people, projects, or organizations.
- Always collect_evidence before answering, so your answer has citations.
- Use <tool_call> tags with JSON content for actions. Format:
  <tool_call>{"name":"tool_name","arguments":{"param":"value"}}</tool_call>
- You can call multiple tools in one turn by including multiple <tool_call> blocks.

## Example turn

I'll search for recent emails about the project.

<tool_call>{"name":"list_sources","arguments":{"content_type":"all"}}</tool_call>
<tool_call>{"name":"keyword_search","arguments":{"pattern":"project","content_type":"raw"}}</tool_call>
"#
        .into()
}

pub(crate) fn build_inner_tools_text() -> String {
    r#"## Available tools

**keyword_search** `{"pattern": "<text>", "content_type": "all|raw|wiki|document|episodic"}`
Search for a text pattern (case-insensitive) across memory files. Returns matching file paths and line previews.

**vector_search** `{"query": "<semantic query>", "source_kind": "chat|email|document", "time_window_days": 30}`
Semantic similarity search over indexed summaries. All params except query are optional.

**entity_search** `{"query": "<name or term>", "kinds": ["person", "email", "url", "handle"]}`
Find entities (people, emails, URLs, handles) in the entity index. kinds is optional.

**list_sources** `{"content_type": "all|raw|wiki|document|episodic"}`
List available content sources and their subdirectories.

**read_content** `{"path": "<relative/path/to/file.md>"}`
Read a specific content file. Path is relative to the content root (e.g. "raw/github-com-example/commits/123.md").

**browse_tree** `{"node_id": "root"}`
Navigate the wiki summary tree. Returns node summary and children. Use "root" to start.

**collect_evidence** `{"items": [{"source": "<path>", "snippet": "<text>", "relevance": "<why relevant>"}]}`
Save evidence snippets for citation in your final answer. Call this as you find relevant information.

**answer** `{"text": "<final synthesized answer with citations>"}`
Return your final answer. Reference collected evidence by source path."#
        .into()
}

// ── Content inventory ────────────────────────────────────────────────────────

pub(crate) fn build_content_inventory(content_root: &Path) -> String {
    let mut parts = Vec::new();

    for (label, subdir) in &[
        ("Raw content", "raw"),
        ("Wiki summaries", "wiki"),
        ("Documents", "document"),
        ("Episodic memories", "episodic"),
    ] {
        let dir = content_root.join(subdir);
        if dir.exists() {
            let count = count_files_recursive(&dir);
            if count > 0 {
                parts.push(format!("- **{label}** ({subdir}/): {count} files"));
            }
        }
    }

    if parts.is_empty() {
        "No content files found.".into()
    } else {
        parts.join("\n")
    }
}

pub(crate) fn count_files_recursive(dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += count_files_recursive(&path);
            } else if path.extension().map_or(false, |e| e == "md") {
                count += 1;
            }
        }
    }
    count
}

// ── Model resolution ─────────────────────────────────────────────────────────

const DEFAULT_SMART_WALK_MODEL: &str = "hint:summarization";

pub(crate) fn resolve_walk_model(config: &Config) -> String {
    // 1. Explicit smart_walk_model config takes priority
    if let Some(ref swm) = config.memory_tree.smart_walk_model {
        if !swm.is_empty() {
            return swm.clone();
        }
    }
    // 2. Default to summarization-v1 (routed through the OpenHuman backend)
    DEFAULT_SMART_WALK_MODEL.to_string()
}

// ── Tool call parser ─────────────────────────────────────────────────────────

pub(crate) fn parse_tool_calls(response: &str) -> (String, Vec<InnerCall>) {
    let mut calls: Vec<InnerCall> = Vec::new();
    let mut text_parts: Vec<&str> = Vec::new();
    let mut remaining: &str = response;

    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";

    loop {
        match remaining.find(OPEN) {
            None => {
                if !remaining.trim().is_empty() && calls.is_empty() {
                    text_parts.push(remaining);
                }
                break;
            }
            Some(start) => {
                let before = &remaining[..start];
                if !before.trim().is_empty() {
                    text_parts.push(before);
                }
                let after_open = &remaining[start + OPEN.len()..];
                match after_open.find(CLOSE) {
                    None => break,
                    Some(close_idx) => {
                        let inner = after_open[..close_idx].trim();
                        if let Some(call) = parse_single_tool_call(inner) {
                            calls.push(call);
                        }
                        remaining = &after_open[close_idx + CLOSE.len()..];
                    }
                }
            }
        }
    }

    let text_before = text_parts.concat();
    (text_before, calls)
}

fn parse_single_tool_call(inner: &str) -> Option<InnerCall> {
    // Primary: JSON format {"name":"...","arguments":{...}}
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(inner) {
        if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
            let args = val
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Object(Default::default()));
            log::debug!(
                "[smart_walk::parse_single_tool_call] json path: tool={} args_keys={}",
                name,
                args.as_object().map(|m| m.len()).unwrap_or(0)
            );
            return Some(InnerCall {
                name: name.to_string(),
                args,
            });
        }
    }
    // Fallback: XML-style <tool_name>name</tool_name><parameters>JSON</parameters>
    if let (Some(name), args) = (
        extract_xml_tag(inner, "tool_name"),
        extract_xml_tag(inner, "parameters"),
    ) {
        log::debug!(
            "[smart_walk::parse_single_tool_call] xml fallback path: tool={} has_params={}",
            name.trim(),
            args.is_some()
        );
        let parsed_args = args
            .and_then(|a| serde_json::from_str::<serde_json::Value>(a.trim()).ok())
            .unwrap_or_else(|| {
                let mut map = serde_json::Map::new();
                for line in inner.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with('<')
                        && !trimmed.starts_with("</")
                        && !trimmed.starts_with("<tool_name")
                        && !trimmed.starts_with("<parameters")
                    {
                        if let Some(tag_end) = trimmed.find('>') {
                            let tag = &trimmed[1..tag_end];
                            if let Some(close) = trimmed.find(&format!("</{tag}>")) {
                                let value = &trimmed[tag_end + 1..close];
                                map.insert(
                                    tag.to_string(),
                                    serde_json::Value::String(value.to_string()),
                                );
                            }
                        }
                    }
                }
                serde_json::Value::Object(map)
            });
        return Some(InnerCall {
            name: name.trim().to_string(),
            args: parsed_args,
        });
    }
    None
}

fn extract_xml_tag<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..].find(&close)? + start;
    Some(&text[start..end])
}

// ── Fallback synthesis ───────────────────────────────────────────────────────

pub(crate) fn synthesize_fallback(trace: &[SmartWalkStep], evidence: &[Evidence]) -> String {
    let mut out = String::new();

    if !evidence.is_empty() {
        out.push_str("Based on the evidence collected:\n\n");
        for (i, ev) in evidence.iter().enumerate() {
            out.push_str(&format!(
                "{}. [{}] {}: {}\n",
                i + 1,
                ev.source_path,
                ev.relevance,
                truncate_chars(&ev.snippet, 150)
            ));
        }
    } else if !trace.is_empty() {
        out.push_str("Could not converge on an answer. Steps taken:\n\n");
        for s in trace {
            out.push_str(&format!(
                "- Turn {}: {} → {}\n",
                s.turn,
                s.action,
                truncate_chars(&s.result_preview, 100)
            ));
        }
    } else {
        out.push_str("Could not converge on an answer — no steps taken.");
    }
    out
}
