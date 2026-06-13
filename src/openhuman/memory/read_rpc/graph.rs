use anyhow::{Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::store::with_connection;
use crate::rpc::RpcOutcome;

// ── wire types ────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GraphMode {
    #[default]
    Tree,
    Contacts,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphNode {
    pub kind: String,
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_range_start_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_range_end_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_basename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_kind: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphExportResponse {
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
    pub content_root_abs: String,
}

// ── graph_export ────────────────────────────────────────────────────────

pub async fn graph_export_rpc(
    config: &Config,
    mode: GraphMode,
) -> Result<RpcOutcome<GraphExportResponse>, String> {
    let cfg = config.clone();
    let resp = tokio::task::spawn_blocking(move || -> Result<GraphExportResponse> {
        let content_root = cfg.memory_tree_content_root();
        let resp = match mode {
            GraphMode::Tree => collect_tree_graph(&cfg)?,
            GraphMode::Contacts => collect_contacts_graph(&cfg)?,
        };
        Ok(GraphExportResponse {
            nodes: resp.0,
            edges: resp.1,
            content_root_abs: content_root.to_string_lossy().to_string(),
        })
    })
    .await
    .map_err(|e| format!("graph_export join error: {e}"))?
    .map_err(|e| format!("graph_export: {e:#}"))?;
    let log = format!(
        "memory_tree::read: graph_export mode={:?} nodes={} edges={} root_hash={}",
        mode,
        resp.nodes.len(),
        resp.edges.len(),
        crate::openhuman::memory::util::redact::redact(&resp.content_root_abs),
    );
    Ok(RpcOutcome::single_log(resp, log))
}

// ── collect_tree_graph ───────────────────────────────────────────────────

fn collect_tree_graph(cfg: &Config) -> Result<(Vec<GraphNode>, Vec<GraphEdge>)> {
    const MAX_TREE_NODES: usize = 10_000;

    struct SummaryRow {
        node: GraphNode,
        tree_scope: String,
        child_ids: Vec<String>,
    }

    let summary_rows = with_connection(cfg, |conn| {
        let mut stmt = conn.prepare(
            "SELECT s.id, s.tree_id, s.tree_kind, t.scope, s.level, s.parent_id,
                    s.child_ids_json, s.time_range_start_ms, s.time_range_end_ms
               FROM mem_tree_summaries s
               JOIN mem_tree_trees t ON t.id = s.tree_id
              WHERE s.deleted = 0
              ORDER BY s.tree_id, s.level, s.sealed_at_ms",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let tree_id: String = row.get(1)?;
                let tree_kind: String = row.get(2)?;
                let tree_scope: String = row.get(3)?;
                let level: i64 = row.get(4)?;
                let parent_id: Option<String> = row.get(5)?;
                let child_ids_json: String = row.get(6)?;
                let time_range_start_ms: i64 = row.get(7)?;
                let time_range_end_ms: i64 = row.get(8)?;
                let child_ids: Vec<String> =
                    serde_json::from_str(&child_ids_json).unwrap_or_default();
                let child_count = child_ids.len() as u32;
                let file_basename = sanitize_basename(&id);
                let label = format!("L{} · {}", level.max(0), tree_scope);
                Ok(SummaryRow {
                    node: GraphNode {
                        kind: "summary".into(),
                        id,
                        label,
                        tree_kind: Some(tree_kind),
                        tree_scope: Some(tree_scope.clone()),
                        tree_id: Some(tree_id),
                        level: Some(level.max(0) as u32),
                        parent_id,
                        child_count: Some(child_count),
                        time_range_start_ms: Some(time_range_start_ms),
                        time_range_end_ms: Some(time_range_end_ms),
                        file_basename: Some(file_basename),
                        entity_kind: None,
                    },
                    tree_scope,
                    child_ids,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("collect tree-mode summary rows")?;
        Ok(rows)
    })?;

    let mut scopes: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for sr in &summary_rows {
        scopes.insert(sr.tree_scope.clone());
    }

    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut source_root_ids: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for scope in &scopes {
        let root_id = format!("source:{scope}");
        let label = scope_display_label(scope);
        source_root_ids.insert(scope.clone(), root_id.clone());
        nodes.push(GraphNode {
            kind: "source".into(),
            id: root_id,
            label,
            tree_kind: None,
            tree_scope: Some(scope.clone()),
            tree_id: None,
            level: None,
            parent_id: None,
            child_count: None,
            time_range_start_ms: None,
            time_range_end_ms: None,
            file_basename: None,
            entity_kind: None,
        });
    }

    let mut summary_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for sr in &summary_rows {
        summary_ids.insert(sr.node.id.clone());
    }

    for sr in &summary_rows {
        let mut node = sr.node.clone();
        let has_valid_parent = node
            .parent_id
            .as_ref()
            .map(|pid| summary_ids.contains(pid))
            .unwrap_or(false);
        if !has_valid_parent {
            node.parent_id = source_root_ids.get(&sr.tree_scope).cloned();
        }
        nodes.push(node);
    }

    let doc_budget = MAX_TREE_NODES.saturating_sub(nodes.len());
    let mut doc_count = 0usize;
    for sr in &summary_rows {
        if doc_count >= doc_budget {
            break;
        }
        if sr.node.level != Some(1) {
            continue;
        }
        if sr
            .child_ids
            .first()
            .map(|c| c.starts_with("summary:"))
            .unwrap_or(false)
        {
            continue;
        }
        for child_id in &sr.child_ids {
            if doc_count >= doc_budget {
                break;
            }
            let label = document_label(child_id);
            nodes.push(GraphNode {
                kind: "chunk".into(),
                id: format!("doc:{}:{}", sr.tree_scope, child_id),
                label,
                tree_kind: None,
                tree_scope: Some(sr.tree_scope.clone()),
                tree_id: None,
                level: None,
                parent_id: Some(sr.node.id.clone()),
                child_count: None,
                time_range_start_ms: sr.node.time_range_start_ms,
                time_range_end_ms: sr.node.time_range_end_ms,
                file_basename: None,
                entity_kind: None,
            });
            doc_count += 1;
        }
    }

    let chunk_budget = MAX_TREE_NODES.saturating_sub(nodes.len());
    if chunk_budget > 0 {
        let chunk_nodes = with_connection(cfg, |conn| {
            let mut stmt = conn.prepare(
                "SELECT c.id, c.parent_summary_id, c.content,
                        c.time_range_start_ms, c.time_range_end_ms, c.source_id
                   FROM mem_tree_chunks c
                  ORDER BY c.timestamp_ms DESC
                  LIMIT ?1",
            )?;
            let rows = stmt
                .query_map(params![chunk_budget as i64], |row| {
                    let id: String = row.get(0)?;
                    let parent_id: Option<String> = row.get(1)?;
                    let content: String = row.get(2)?;
                    let time_range_start_ms: i64 = row.get(3)?;
                    let time_range_end_ms: i64 = row.get(4)?;
                    let source_id: String = row.get(5)?;
                    let label = content
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(72)
                        .collect::<String>();
                    Ok((
                        GraphNode {
                            kind: "chunk".into(),
                            id,
                            label,
                            tree_kind: None,
                            tree_scope: None,
                            tree_id: None,
                            level: None,
                            parent_id: parent_id.filter(|s| !s.is_empty()),
                            child_count: None,
                            time_range_start_ms: Some(time_range_start_ms),
                            time_range_end_ms: Some(time_range_end_ms),
                            file_basename: None,
                            entity_kind: None,
                        },
                        source_id,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("collect tree-mode leaf chunk rows")?;
            Ok(rows)
        })?;

        for (chunk, _source_id) in chunk_nodes {
            nodes.push(chunk);
        }
    }

    Ok((nodes, Vec::new()))
}

fn scope_display_label(scope: &str) -> String {
    if scope.starts_with("github:") {
        let repo = scope.strip_prefix("github:").unwrap_or(scope);
        format!("GitHub · {repo}")
    } else if scope.starts_with("gmail:") {
        let account = scope
            .strip_prefix("gmail:")
            .unwrap_or(scope)
            .replace("-at-", "@")
            .replace("-dot-", ".");
        format!("Gmail · {account}")
    } else if scope.starts_with("slack:") {
        let channel = scope.strip_prefix("slack:").unwrap_or(scope);
        format!("Slack · {channel}")
    } else {
        scope.to_string()
    }
}

fn document_label(child_id: &str) -> String {
    if let Some(sha) = child_id.strip_prefix("commit:") {
        format!("commit {}", &sha[..sha.len().min(8)])
    } else if let Some(n) = child_id.strip_prefix("issue:") {
        format!("issue #{n}")
    } else if let Some(n) = child_id.strip_prefix("pr:") {
        format!("PR #{n}")
    } else {
        child_id.chars().take(40).collect()
    }
}

#[allow(dead_code)]
pub(super) fn source_id_to_scope(source_id: &str) -> String {
    let parts: Vec<&str> = source_id.splitn(3, ':').collect();
    if parts.len() >= 2 {
        format!("{}:{}", parts[0], parts[1])
    } else {
        source_id.to_string()
    }
}

// ── collect_contacts_graph ───────────────────────────────────────────────

fn collect_contacts_graph(cfg: &Config) -> Result<(Vec<GraphNode>, Vec<GraphEdge>)> {
    const MAX_CHUNK_NODES: usize = 1500;
    const MAX_EDGES: usize = 4000;

    with_connection(cfg, |conn| {
        let mut chunk_stmt = conn.prepare(
            "SELECT c.id, c.timestamp_ms, c.content
               FROM mem_tree_chunks c
              WHERE c.id IN (
                    SELECT DISTINCT node_id
                      FROM mem_tree_entity_index
                     WHERE entity_kind = 'person'
              )
              ORDER BY c.timestamp_ms DESC
              LIMIT ?1",
        )?;
        let chunks: Vec<(String, i64, String)> = chunk_stmt
            .query_map(params![MAX_CHUNK_NODES as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<_>>()
            .context("collect contacts-mode chunk rows")?;

        let chunk_ids: Vec<String> = chunks.iter().map(|(id, _, _)| id.clone()).collect();

        let edges: Vec<(String, String, String)> = if chunk_ids.is_empty() {
            Vec::new()
        } else {
            let placeholders = std::iter::repeat("?")
                .take(chunk_ids.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT entity_id, node_id, surface
                   FROM mem_tree_entity_index
                  WHERE entity_kind = 'person'
                    AND node_kind = 'leaf'
                    AND node_id IN ({placeholders})
                  ORDER BY timestamp_ms DESC
                  LIMIT ?"
            );
            let mut bind: Vec<rusqlite::types::Value> = chunk_ids
                .iter()
                .map(|s| rusqlite::types::Value::Text(s.clone()))
                .collect();
            bind.push(rusqlite::types::Value::Integer(MAX_EDGES as i64));
            let mut mention_stmt = conn.prepare(&sql)?;
            let rows = mention_stmt
                .query_map(rusqlite::params_from_iter(bind), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
                .context("collect contacts-mode mentions")?;
            rows
        };

        let mut edges_out: Vec<GraphEdge> = Vec::with_capacity(edges.len());
        let mut contacts: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for (entity_id, node_id, surface) in edges {
            contacts.entry(entity_id.clone()).or_insert(surface);
            edges_out.push(GraphEdge {
                from: node_id,
                to: entity_id,
            });
        }

        let mut nodes: Vec<GraphNode> = Vec::with_capacity(chunks.len() + contacts.len());
        for (id, ts, preview) in chunks {
            let label = preview
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(72)
                .collect::<String>();
            nodes.push(GraphNode {
                kind: "chunk".into(),
                id,
                label,
                tree_kind: None,
                tree_scope: None,
                tree_id: None,
                level: None,
                parent_id: None,
                child_count: None,
                time_range_start_ms: Some(ts),
                time_range_end_ms: Some(ts),
                file_basename: None,
                entity_kind: None,
            });
        }
        for (entity_id, surface) in contacts {
            nodes.push(GraphNode {
                kind: "contact".into(),
                id: entity_id,
                label: surface,
                tree_kind: None,
                tree_scope: None,
                tree_id: None,
                level: None,
                parent_id: None,
                child_count: None,
                time_range_start_ms: None,
                time_range_end_ms: None,
                file_basename: None,
                entity_kind: Some("person".into()),
            });
        }
        Ok((nodes, edges_out))
    })
}

pub fn sanitize_basename(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            other => other,
        })
        .collect()
}
