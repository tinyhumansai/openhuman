//! Tool call dispatch for the smart_walk inner loop.
//!
//! Each `dispatch_*` function handles one named inner tool and returns
//! `(args_summary, result_text, is_final_answer, answer_text)`.

use crate::openhuman::config::Config;
use crate::openhuman::memory::query::smart_walk::prompts::InnerCall;
use crate::openhuman::memory::query::smart_walk::types::{
    Evidence, MAX_EVIDENCE_ITEMS, MAX_FILE_READ_BYTES, MAX_KEYWORD_RESULTS,
};
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::memory_tree::retrieval;
use crate::openhuman::memory_tree::score::extract::EntityKind;
use crate::openhuman::memory_tree::tree_runtime::store::{read_children, read_node};
use std::path::{Path, PathBuf};

// ── Top-level dispatcher ─────────────────────────────────────────────────────

pub(crate) async fn dispatch_call(
    config: &Config,
    namespace: &str,
    content_root: &Path,
    call: &InnerCall,
    evidence: &mut Vec<Evidence>,
) -> (String, String, bool, String) {
    match call.name.as_str() {
        "keyword_search" => {
            let cr = content_root.to_path_buf();
            let c = call.clone();
            tokio::task::spawn_blocking(move || dispatch_keyword_search(&cr, &c))
                .await
                .unwrap_or_else(|e| (String::new(), format!("error: {e}"), false, String::new()))
        }
        "entity_search" => dispatch_entity_search(config, call).await,
        "list_sources" => {
            let cr = content_root.to_path_buf();
            let c = call.clone();
            tokio::task::spawn_blocking(move || dispatch_list_sources(&cr, &c))
                .await
                .unwrap_or_else(|e| (String::new(), format!("error: {e}"), false, String::new()))
        }
        "read_content" => {
            let cr = content_root.to_path_buf();
            let c = call.clone();
            tokio::task::spawn_blocking(move || dispatch_read_content(&cr, &c))
                .await
                .unwrap_or_else(|e| (String::new(), format!("error: {e}"), false, String::new()))
        }
        "browse_tree" => dispatch_browse_tree(config, namespace, call).await,
        "collect_evidence" => dispatch_collect_evidence(call, evidence),
        "answer" => dispatch_answer(call),
        "vector_search" => dispatch_vector_search(config, call).await,
        other => {
            log::warn!("[smart_walk] unknown action: {other}");
            (
                format!("action={other}"),
                format!(
                    "unknown action '{other}'. Valid: keyword_search, entity_search, \
                     list_sources, read_content, browse_tree, vector_search, \
                     collect_evidence, answer"
                ),
                false,
                String::new(),
            )
        }
    }
}

// ── keyword_search ───────────────────────────────────────────────────────────

pub(crate) fn dispatch_keyword_search(
    content_root: &Path,
    call: &InnerCall,
) -> (String, String, bool, String) {
    let pattern = call
        .args
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let content_type = call
        .args
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("all");

    if pattern.is_empty() {
        return (
            "pattern=<empty>".into(),
            "error: keyword_search requires a non-empty pattern".into(),
            false,
            String::new(),
        );
    }

    log::debug!(
        "[smart_walk] keyword_search pattern={} content_type={}",
        pattern,
        content_type
    );

    let args_summary = format!("pattern=\"{}\" type={}", pattern, content_type);

    let search_dirs: Vec<PathBuf> = match content_type {
        "raw" => vec![content_root.join("raw")],
        "wiki" => vec![content_root.join("wiki")],
        "document" => vec![content_root.join("document")],
        "episodic" => vec![content_root.join("episodic")],
        _ => vec![
            content_root.join("raw"),
            content_root.join("wiki"),
            content_root.join("document"),
            content_root.join("episodic"),
        ],
    };

    let pattern_lower = pattern.to_lowercase();
    let mut results: Vec<String> = Vec::new();

    for dir in &search_dirs {
        if !dir.exists() {
            continue;
        }
        search_dir_recursive(dir, &pattern_lower, &mut results, content_root);
        if results.len() >= MAX_KEYWORD_RESULTS {
            break;
        }
    }

    results.truncate(MAX_KEYWORD_RESULTS);

    if results.is_empty() {
        (
            args_summary,
            format!("no matches for pattern \"{}\"", pattern),
            false,
            String::new(),
        )
    } else {
        let count = results.len();
        (
            args_summary,
            format!("{count} matches:\n{}", results.join("\n")),
            false,
            String::new(),
        )
    }
}

pub(crate) fn search_dir_recursive(
    dir: &Path,
    pattern: &str,
    results: &mut Vec<String>,
    content_root: &Path,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        if results.len() >= MAX_KEYWORD_RESULTS {
            return;
        }

        let path = entry.path();
        if path.is_dir() {
            search_dir_recursive(&path, pattern, results, content_root);
        } else if path.extension().map_or(false, |e| e == "md") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if content.to_lowercase().contains(pattern) {
                    let rel = path
                        .strip_prefix(content_root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string();

                    let line_match = content
                        .lines()
                        .find(|l| l.to_lowercase().contains(pattern))
                        .unwrap_or("")
                        .trim();
                    let preview: String = line_match.chars().take(120).collect();
                    results.push(format!("  [{rel}] {preview}"));
                }
            }
        }
    }
}

// ── entity_search ────────────────────────────────────────────────────────────

async fn dispatch_entity_search(
    config: &Config,
    call: &InnerCall,
) -> (String, String, bool, String) {
    let query = call
        .args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let kinds: Option<Vec<EntityKind>> =
        call.args
            .get("kinds")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .filter_map(|s| EntityKind::parse(s).ok())
                    .collect()
            });

    if query.is_empty() {
        return (
            "query=<empty>".into(),
            "error: entity_search requires a non-empty query".into(),
            false,
            String::new(),
        );
    }

    log::debug!(
        "[smart_walk] entity_search query={} kinds={:?}",
        query,
        kinds
            .as_ref()
            .map(|ks| ks.iter().map(|k| k.as_str()).collect::<Vec<_>>())
    );
    let args_summary = format!(
        "query=\"{}\" kinds={:?}",
        query,
        kinds
            .as_ref()
            .map(|ks| ks.iter().map(|k| k.as_str()).collect::<Vec<_>>())
    );

    match retrieval::search_entities(config, &query, kinds, 10).await {
        Ok(matches) => {
            if matches.is_empty() {
                (
                    args_summary,
                    format!("no entities matching \"{}\"", query),
                    false,
                    String::new(),
                )
            } else {
                let formatted: Vec<String> = matches
                    .iter()
                    .map(|m| {
                        format!(
                            "  [{}] kind={} surface=\"{}\" mentions={} last_seen={}",
                            m.canonical_id,
                            m.kind.as_str(),
                            m.surface,
                            m.mention_count,
                            m.last_seen_ms
                        )
                    })
                    .collect();
                (
                    args_summary,
                    format!(
                        "{} entities found:\n{}",
                        formatted.len(),
                        formatted.join("\n")
                    ),
                    false,
                    String::new(),
                )
            }
        }
        Err(e) => (
            args_summary,
            format!("entity search error: {e}"),
            false,
            String::new(),
        ),
    }
}

// ── list_sources ─────────────────────────────────────────────────────────────

pub(crate) fn dispatch_list_sources(
    content_root: &Path,
    call: &InnerCall,
) -> (String, String, bool, String) {
    let content_type = call
        .args
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("all");

    log::debug!("[smart_walk] list_sources type={}", content_type);
    let args_summary = format!("type={}", content_type);

    let mut listing = Vec::new();

    let types_to_scan: Vec<&str> = match content_type {
        "all" => vec!["raw", "wiki", "document", "episodic"],
        t => vec![t],
    };

    for ctype in types_to_scan {
        let dir = content_root.join(ctype);
        if !dir.exists() {
            listing.push(format!("  {ctype}/: (empty)"));
            continue;
        }

        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                let mut subdirs: Vec<String> = entries
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect();
                subdirs.sort();

                if subdirs.is_empty() {
                    listing.push(format!("  {ctype}/: (no subdirectories)"));
                } else {
                    let count = subdirs.len();
                    let preview: Vec<&str> = subdirs.iter().map(|s| s.as_str()).take(10).collect();
                    listing.push(format!(
                        "  {ctype}/ ({count} sources): {}{}",
                        preview.join(", "),
                        if count > 10 { ", ..." } else { "" }
                    ));
                }
            }
            Err(e) => listing.push(format!("  {ctype}/: error: {e}")),
        }
    }

    (
        args_summary,
        format!("Content sources:\n{}", listing.join("\n")),
        false,
        String::new(),
    )
}

// ── read_content ─────────────────────────────────────────────────────────────

pub(crate) fn dispatch_read_content(
    content_root: &Path,
    call: &InnerCall,
) -> (String, String, bool, String) {
    let path_str = call
        .args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if path_str.is_empty() {
        return (
            "path=<empty>".into(),
            "error: read_content requires a non-empty path".into(),
            false,
            String::new(),
        );
    }

    let requested = Path::new(&path_str);
    if requested.is_absolute() || path_str.contains("..") {
        return (
            format!("path={path_str}"),
            "error: path must stay within the content root".into(),
            false,
            String::new(),
        );
    }

    log::debug!("[smart_walk] read_content path={}", path_str);

    let full_path = content_root.join(requested);
    if !full_path.exists() {
        return (
            format!("path={path_str}"),
            format!("file not found: {path_str}"),
            false,
            String::new(),
        );
    }

    let canonical_root = match content_root.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return (
                format!("path={path_str}"),
                format!("error resolving content root: {e}"),
                false,
                String::new(),
            );
        }
    };
    let canonical_path = match full_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return (
                format!("path={path_str}"),
                format!("error resolving path: {e}"),
                false,
                String::new(),
            );
        }
    };
    if !canonical_path.starts_with(&canonical_root) {
        return (
            format!("path={path_str}"),
            "error: path escapes content root".into(),
            false,
            String::new(),
        );
    }

    match std::fs::read_to_string(&canonical_path) {
        Ok(content) => {
            let truncated: String = content.chars().take(MAX_FILE_READ_BYTES).collect();
            let was_truncated = content.len() > MAX_FILE_READ_BYTES;
            let suffix = if was_truncated {
                format!("\n\n[...truncated, {} total chars]", content.len())
            } else {
                String::new()
            };
            (
                format!("path={path_str}"),
                format!("{truncated}{suffix}"),
                false,
                String::new(),
            )
        }
        Err(e) => (
            format!("path={path_str}"),
            format!("error reading: {e}"),
            false,
            String::new(),
        ),
    }
}

// ── browse_tree ──────────────────────────────────────────────────────────────

async fn dispatch_browse_tree(
    config: &Config,
    namespace: &str,
    call: &InnerCall,
) -> (String, String, bool, String) {
    let node_id = call
        .args
        .get("node_id")
        .and_then(|v| v.as_str())
        .unwrap_or("root")
        .to_string();

    log::debug!("[smart_walk] browse_tree node_id={}", node_id);

    let config_owned = config.clone();
    let ns_owned = namespace.to_string();
    let id_owned = node_id.clone();

    let result = tokio::task::spawn_blocking(move || {
        let node = match read_node(&config_owned, &ns_owned, &id_owned) {
            Ok(Some(n)) => n,
            Ok(None) => return format!("unknown node: {id_owned}"),
            Err(e) => return format!("error reading node {id_owned}: {e}"),
        };

        let children = match read_children(&config_owned, &ns_owned, &id_owned) {
            Ok(c) => c,
            Err(_) => vec![],
        };

        let mut out = format!(
            "Node: {} (level={:?})\nSummary: {}\n",
            node.node_id, node.level, node.summary
        );

        if children.is_empty() {
            out.push_str("Children: (none — leaf node)\n");
        } else {
            out.push_str(&format!("Children ({}):\n", children.len()));
            for c in &children {
                let preview: String = c.summary.chars().take(100).collect();
                out.push_str(&format!(
                    "  - id={} level={:?}: {}\n",
                    c.node_id, c.level, preview
                ));
            }
        }
        out
    })
    .await
    .unwrap_or_else(|_| format!("error building context for node {node_id}"));

    (format!("node_id={node_id}"), result, false, String::new())
}

// ── vector_search ────────────────────────────────────────────────────────────

async fn dispatch_vector_search(
    config: &Config,
    call: &InnerCall,
) -> (String, String, bool, String) {
    use crate::openhuman::memory::query::smart_walk::types::truncate_chars;

    let query = call
        .args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let source_kind = call
        .args
        .get("source_kind")
        .and_then(|v| v.as_str())
        .and_then(|s| match s {
            "chat" => Some(SourceKind::Chat),
            "email" => Some(SourceKind::Email),
            "document" => Some(SourceKind::Document),
            _ => None,
        });

    let time_window_days = call
        .args
        .get("time_window_days")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);

    if query.is_empty() {
        return (
            "query=<empty>".into(),
            "error: vector_search requires a non-empty query".into(),
            false,
            String::new(),
        );
    }

    log::debug!(
        "[smart_walk] vector_search query={} source_kind={:?} window_days={:?}",
        query,
        source_kind,
        time_window_days
    );
    let args_summary = format!(
        "query=\"{}\" kind={:?} window={:?}",
        truncate_chars(&query, 40),
        source_kind,
        time_window_days
    );

    match retrieval::query_source(
        config,
        None,
        source_kind,
        time_window_days,
        Some(&query),
        10,
    )
    .await
    {
        Ok(resp) => {
            if resp.hits.is_empty() {
                (
                    args_summary,
                    format!("no vector matches for \"{}\"", query),
                    false,
                    String::new(),
                )
            } else {
                let formatted: Vec<String> = resp
                    .hits
                    .iter()
                    .map(|h| {
                        let preview: String = h.content.chars().take(120).collect();
                        format!("  [{}] (score={:.2}) {}", h.node_id, h.score, preview)
                    })
                    .collect();
                (
                    args_summary,
                    format!(
                        "{} semantic matches:\n{}",
                        formatted.len(),
                        formatted.join("\n")
                    ),
                    false,
                    String::new(),
                )
            }
        }
        Err(e) => (
            args_summary,
            format!("vector search error: {e}"),
            false,
            String::new(),
        ),
    }
}

// ── collect_evidence ─────────────────────────────────────────────────────────

pub(crate) fn dispatch_collect_evidence(
    call: &InnerCall,
    evidence: &mut Vec<Evidence>,
) -> (String, String, bool, String) {
    let items = call
        .args
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if items.is_empty() {
        return (
            "items=[]".into(),
            "error: collect_evidence requires non-empty items array".into(),
            false,
            String::new(),
        );
    }

    let mut added = 0;
    for item in &items {
        if evidence.len() >= MAX_EVIDENCE_ITEMS {
            break;
        }
        let source_path = item
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let snippet = item
            .get("snippet")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let relevance = item
            .get("relevance")
            .and_then(|v| v.as_str())
            .unwrap_or("relevant")
            .to_string();

        if !snippet.is_empty() {
            evidence.push(Evidence {
                source_path,
                snippet,
                relevance,
            });
            added += 1;
        }
    }

    log::debug!(
        "[smart_walk] collect_evidence added={} total={}",
        added,
        evidence.len()
    );

    (
        format!("{added} items"),
        format!(
            "collected {added} evidence items (total: {})",
            evidence.len()
        ),
        false,
        String::new(),
    )
}

// ── answer ───────────────────────────────────────────────────────────────────

pub(crate) fn dispatch_answer(call: &InnerCall) -> (String, String, bool, String) {
    let text = call
        .args
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    log::debug!("[smart_walk] answer text_len={}", text.len());
    ("(final answer)".into(), text.clone(), true, text)
}
