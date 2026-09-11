use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::ChunkQuery;
use crate::openhuman::memory::api::tree::{TreeLeaf, TreeSummary};
use crate::rpc::RpcOutcome;

// `MemoryGraph` in the contract is the key/value and relation tier, a different
// graph from the one exported here: the summary forest and its leaf chunks,
// laid out for the Memory tab. Both collectors below are ordinary contract
// reads — `MemoryTree::{summary_forest, recent_leaves}` for the forest, and
// `MemoryChunks::list_chunks` + `MemoryEntities::chunk_entities` for the
// contacts view — so neither reaches the engine's tables any more (#5560).
//
// All four forwarders now exist in `modules::memory` (`summary_forest`,
// `recent_leaves`, `chunk_entities` — added in this PR alongside the
// `modules::registry` re-pin to `ARTIFACT_CAPABILITIES_PIN = "1.5.0"`). Errors
// are propagated rather than degraded to an empty graph, deliberately:
// `summary_forest`'s own contract refuses to answer an empty forest for exactly
// this reason, and a Memory tab that renders "no memories" because a method is
// missing is the failure nobody chases.

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

/// Label cap for a chunk node, in characters. Unchanged by the move onto the
/// contract: `TreeLeaf::preview` is capped at 200 by the driver, so this is
/// still the cut the UI sees.
const CHUNK_LABEL_CHARS: usize = 72;

// ── graph_export ────────────────────────────────────────────────────────

pub async fn graph_export_rpc(
    config: &Config,
    mode: GraphMode,
) -> Result<RpcOutcome<GraphExportResponse>, String> {
    log::debug!("[memory_tree::read::graph_export] mode={mode:?}");

    // No `spawn_blocking`: every read below is a driver call, the driver owns
    // whether its own reads block, and the module's do not run on this thread
    // at all. The only other work here is `memory_tree_content_root`, which is
    // a path join.
    let content_root = config.memory_tree_content_root();
    let (nodes, edges) = match mode {
        GraphMode::Tree => collect_tree_graph(config).await,
        GraphMode::Contacts => collect_contacts_graph(config).await,
    }
    .map_err(|e| format!("graph_export: {e}"))?;

    let resp = GraphExportResponse {
        nodes,
        edges,
        content_root_abs: content_root.to_string_lossy().to_string(),
    };
    let log = format!(
        "memory_tree::read: graph_export mode={:?} nodes={} edges={} root_hash={}",
        mode,
        resp.nodes.len(),
        resp.edges.len(),
        crate::openhuman::util::redact::redact(&resp.content_root_abs),
    );
    Ok(RpcOutcome::single_log(resp, log))
}

// ── collect_tree_graph ───────────────────────────────────────────────────

async fn collect_tree_graph(cfg: &Config) -> Result<(Vec<GraphNode>, Vec<GraphEdge>), String> {
    const MAX_TREE_NODES: usize = 10_000;

    let binding = crate::openhuman::memory::binding::for_config(cfg)?;
    // Resolved once and held across both calls: the forest skeleton and its
    // bottom edge are two members of the same family, and re-resolving between
    // them could straddle a rebind.
    let tree = binding.provider().as_tree();
    if tree.is_none() {
        // Read-only, so an empty graph is the honest answer: a driver with no
        // tree tier holds no forest to walk. Contrast `summary_forest`'s own
        // `Unsupported`, which is propagated below — a driver that *has* trees
        // and reports none would be a lie rendered as an empty store.
        log::debug!(
            "[memory_tree::read::graph_tree] driver '{}' does not serve Tree; reporting empty",
            binding.driver_id()
        );
    }

    // `MAX_TREE_NODES` is the whole graph's node budget, so it is also the only
    // honest ceiling to ask the forest for: summaries are emitted before the
    // document and leaf tails, and anything past the budget could not be
    // rendered anyway. The driver clamps further to its own cap.
    let forest = match tree {
        Some(tree) => tree
            .summary_forest(MAX_TREE_NODES, None)
            .await
            .map_err(|e| format!("summary_forest: {e}"))?,
        None => Default::default(),
    };
    if forest.truncated {
        // Worth saying out loud: the walk is tree-major, so a truncated forest
        // is missing whole trees off the tail rather than being thinned evenly.
        log::debug!(
            "[memory_tree::read::graph_tree] summary_forest truncated at limit={MAX_TREE_NODES}; whole trees are missing from the tail"
        );
    }
    log::debug!(
        "[memory_tree::read::graph_tree] summaries={} truncated={}",
        forest.summaries.len(),
        forest.truncated
    );

    let mut nodes = shape_summary_nodes(forest.summaries, MAX_TREE_NODES);

    let chunk_budget = MAX_TREE_NODES.saturating_sub(nodes.len());
    // The guard is load-bearing, not cosmetic: the driver clamps a listing
    // limit into `1..=cap`, so asking for zero leaves would return one.
    if chunk_budget > 0 {
        let leaves = match tree {
            Some(tree) => tree
                .recent_leaves(chunk_budget, None)
                .await
                .map_err(|e| format!("recent_leaves: {e}"))?,
            None => Vec::new(),
        };
        log::debug!(
            "[memory_tree::read::graph_tree] leaves={} budget={chunk_budget}",
            leaves.len()
        );
        for leaf in leaves {
            let TreeLeaf {
                chunk_id,
                parent_summary_id,
                // Selected by the read this replaced and discarded there too:
                // a leaf node draws its edge from its summary, not its source.
                source_id: _,
                preview,
                time_range_start,
                time_range_end,
            } = leaf;
            let label = preview.chars().take(CHUNK_LABEL_CHARS).collect::<String>();
            nodes.push(GraphNode {
                kind: "chunk".into(),
                id: chunk_id,
                label,
                tree_kind: None,
                tree_scope: None,
                tree_id: None,
                level: None,
                // The contract's driver already drops the empty string; kept
                // here so a driver that does not cannot draw an edge to a node
                // named "".
                parent_id: parent_summary_id.filter(|s| !s.is_empty()),
                child_count: None,
                time_range_start_ms: Some(time_range_start.timestamp_millis()),
                time_range_end_ms: Some(time_range_end.timestamp_millis()),
                file_basename: None,
                entity_kind: None,
            });
        }
    }

    Ok((nodes, Vec::new()))
}

/// Shape the forest's summary nodes into graph nodes: a synthetic source root
/// per scope, every summary re-parented onto its root when its real parent is
/// not itself in the forest, and a document leaf per child id of an L1 summary
/// that sealed over raw items rather than other summaries.
///
/// Split out of [`collect_tree_graph`] because it is the whole of what this
/// export decides. The driver supplies the forest and nothing else: which
/// nodes exist, how they are labelled, what is a root, what is an orphan and
/// what becomes a document leaf are all decided here. Keeping it a pure
/// function of the forest is what lets those rules be asserted directly —
/// `summary_forest` is served by the memory module now, so a test that reached
/// them through `graph_export_rpc` would need a loaded module to say anything
/// about node shape (see graph_tests.rs).
///
/// `max_nodes` bounds the whole result, document leaves included; the summary
/// nodes are emitted first and the document tail is what gets cut.
fn shape_summary_nodes(summaries: Vec<TreeSummary>, max_nodes: usize) -> Vec<GraphNode> {
    struct SummaryRow {
        node: GraphNode,
        tree_scope: String,
        child_ids: Vec<String>,
    }

    // Destructured exhaustively rather than field-picked: a field added to
    // `TreeSummary` upstream then fails to compile here instead of silently
    // going unrendered.
    let summary_rows: Vec<SummaryRow> = summaries
        .into_iter()
        .map(|summary| {
            let TreeSummary {
                id,
                tree_id,
                tree_kind,
                tree_scope,
                level,
                parent_id,
                child_ids,
                time_range_start,
                time_range_end,
            } = summary;
            let child_count = child_ids.len() as u32;
            let file_basename = sanitize_basename(&id);
            let label = format!("L{level} · {tree_scope}");
            SummaryRow {
                node: GraphNode {
                    kind: "summary".into(),
                    id,
                    label,
                    tree_kind: Some(tree_kind),
                    tree_scope: Some(tree_scope.clone()),
                    tree_id: Some(tree_id),
                    level: Some(level),
                    // Already empty-filtered by the driver; the reassignment
                    // below treats an unresolvable parent as a root either way.
                    parent_id,
                    child_count: Some(child_count),
                    time_range_start_ms: Some(time_range_start.timestamp_millis()),
                    time_range_end_ms: Some(time_range_end.timestamp_millis()),
                    file_basename: Some(file_basename),
                    entity_kind: None,
                },
                tree_scope,
                child_ids,
            }
        })
        .collect();

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

    let doc_budget = max_nodes.saturating_sub(nodes.len());
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

    nodes
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

async fn collect_contacts_graph(cfg: &Config) -> Result<(Vec<GraphNode>, Vec<GraphEdge>), String> {
    const MAX_CHUNK_NODES: usize = 1500;
    const MAX_EDGES: usize = 4000;
    const PERSON_KIND: &str = "person";

    let binding = crate::openhuman::memory::binding::for_config(cfg)?;

    // `entity_kinds` alone, and that is a deliberate choice rather than the
    // only one available. `entity_ids` is a *second, independent* `EXISTS`
    // clause, so setting both would ask for a chunk holding one of the listed
    // entities **and** one entity of a listed kind — two index rows that need
    // not be the same observation. The question here is simply "does this
    // chunk mention a person", which is this predicate on its own.
    let query = ChunkQuery {
        entity_kinds: vec![PERSON_KIND.to_string()],
        limit: Some(MAX_CHUNK_NODES),
        // `exclude_dropped` stays at its `false` default: the read this
        // replaces had no lifecycle filter. It is moot in practice — a dropped
        // chunk has its entity rows cleared, so it cannot match the predicate
        // above — but "moot" is not a reason to change what is asked for.
        ..Default::default()
    };
    let chunks = match binding.provider().as_chunks() {
        Some(chunks) => chunks
            .list_chunks(&query, None)
            .await
            .map_err(|e| format!("list_chunks: {e}"))?,
        None => {
            log::debug!(
                "[memory_tree::read::graph_contacts] driver '{}' does not serve Chunks; reporting empty",
                binding.driver_id()
            );
            Vec::new()
        }
    };
    let chunk_ids: Vec<String> = chunks.iter().map(|chunk| chunk.id.clone()).collect();

    // One batched call for every edge, not one per chunk: at `MAX_CHUNK_NODES`
    // the per-chunk shape is 1 500 round trips over a bus to read one index,
    // which is the fan-out the batch amendment exists to kill.
    //
    // The empty case short-circuits rather than sending an empty list. Nothing
    // in the contract promises what an empty batch selects, and the sibling
    // rule for `ChunkQuery`'s plural predicates is that empty means
    // *unfiltered* — a computed candidate set that came back empty must skip
    // the query, never hand it over as "no constraint".
    let person_kinds = [PERSON_KIND.to_string()];
    let occurrences = if chunk_ids.is_empty() {
        Vec::new()
    } else {
        match binding.provider().as_entities() {
            Some(entities) => entities
                .chunk_entities(&chunk_ids, Some(&person_kinds[..]))
                .await
                .map_err(|e| format!("chunk_entities: {e}"))?,
            None => {
                log::debug!(
                    "[memory_tree::read::graph_contacts] driver '{}' does not serve Entities; reporting no edges",
                    binding.driver_id()
                );
                Vec::new()
            }
        }
    };
    log::debug!(
        "[memory_tree::read::graph_contacts] chunks={} occurrences={}",
        chunk_ids.len(),
        occurrences.len()
    );

    // The result is FLAT and every row names its own chunk, so it is grouped by
    // `row.chunk_id` and never indexed by position against the ids sent: a
    // chunk the extractor has not reached contributes no row at all, and one
    // chunk contributes a row per distinct `(entity, surface)`.
    let mut by_chunk: std::collections::HashMap<String, Vec<(String, String)>> =
        std::collections::HashMap::new();
    for row in occurrences {
        by_chunk
            .entry(row.chunk_id)
            .or_default()
            .push((row.occurrence.entity_id, row.occurrence.surface));
    }

    // Walked in the order the listing returned — newest chunk first — so the
    // `MAX_EDGES` cut keeps the most recent mentions, which is what the
    // `ORDER BY timestamp_ms DESC LIMIT` this replaces did. That equivalence is
    // exact rather than approximate: an index row is stamped with its own
    // chunk's timestamp, so ordering by chunk recency *is* ordering by mention
    // time. `chunk_entities` itself sorts by chunk id, which would otherwise
    // have made the cut arbitrary.
    let mut edges_out: Vec<GraphEdge> = Vec::new();
    let mut contacts: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    'walk: for chunk in &chunks {
        let Some(rows) = by_chunk.remove(&chunk.id) else {
            continue;
        };
        for (entity_id, surface) in rows {
            if edges_out.len() >= MAX_EDGES {
                log::debug!(
                    "[memory_tree::read::graph_contacts] edge budget {MAX_EDGES} reached; older mentions dropped"
                );
                break 'walk;
            }
            contacts.entry(entity_id.clone()).or_insert(surface);
            edges_out.push(GraphEdge {
                from: chunk.id.clone(),
                to: entity_id,
            });
        }
    }

    let mut nodes: Vec<GraphNode> = Vec::with_capacity(chunks.len() + contacts.len());
    for chunk in chunks {
        // The same `content` column the read this replaces selected, so the
        // label is byte-identical: `MemoryChunks` hands back the stored body,
        // or its ≤500-char preview for a chunk whose body went to the vault.
        let label = chunk
            .content
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(CHUNK_LABEL_CHARS)
            .collect::<String>();
        let ts = chunk.metadata.timestamp.timestamp_millis();
        nodes.push(GraphNode {
            kind: "chunk".into(),
            id: chunk.id,
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
            entity_kind: Some(PERSON_KIND.into()),
        });
    }
    Ok((nodes, edges_out))
}

pub fn sanitize_basename(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            other => other,
        })
        .collect()
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
