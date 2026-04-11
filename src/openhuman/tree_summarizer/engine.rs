//! Core summarization engine: ingest raw data, summarize into hour leaves,
//! and propagate summaries upward through the tree.

use anyhow::{Context, Result};
use chrono::{DateTime, Timelike, Utc};
use std::collections::BTreeMap;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::openhuman::providers::traits::Provider;
use crate::openhuman::tree_summarizer::store;
use crate::openhuman::tree_summarizer::types::{
    derive_node_ids, derive_parent_id, estimate_tokens, level_from_node_id, NodeLevel, TreeNode,
    TreeStatus,
};

/// The model hint passed to the provider for summarization tasks.
const SUMMARIZATION_MODEL: &str = "hint:fast";
const SUMMARIZATION_TEMP: f64 = 0.3;

/// Maximum characters for a summary response (hard limit enforced after LLM call).
/// Set to 4x the Root token budget as a generous upper bound.
const MAX_SUMMARY_CHARS: usize = 20_000 * 4;

// ── Public API ─────────────────────────────────────────────────────────

/// Run the summarization job for a given namespace.
///
/// 1. Drains the ingestion buffer.
/// 2. Groups buffered entries by their original hour (from filename timestamps).
/// 3. Summarizes each hour group into its own hour leaf.
/// 4. Propagates summaries upward through day → month → year → root.
///
/// Returns the last hour leaf node created, or `None` if the buffer was empty.
pub async fn run_summarization(
    config: &Config,
    provider: &dyn Provider,
    namespace: &str,
    _ts: DateTime<Utc>,
) -> Result<Option<TreeNode>> {
    // Read buffer entries non-destructively; we only delete after durable writes.
    let buffered = store::buffer_read(config, namespace)?;
    if buffered.is_empty() {
        tracing::debug!("[tree_summarizer] no buffered data for namespace '{namespace}', skipping");
        return Ok(None);
    }

    let buffer_filenames: Vec<String> = buffered.iter().map(|(name, _)| name.clone()).collect();

    tracing::debug!(
        "[tree_summarizer] starting summarization for namespace '{}' with {} buffer entries",
        namespace,
        buffered.len()
    );

    // Group buffered entries by hour using their buffer filename timestamps.
    let hour_groups = group_by_hour(&buffered);

    tracing::debug!(
        "[tree_summarizer] grouped into {} distinct hours",
        hour_groups.len()
    );

    // Track all ancestor IDs to propagate after all hour leaves are written.
    let mut all_propagation_ids: Vec<(String, NodeLevel)> = Vec::new();
    let mut last_hour_node: Option<TreeNode> = None;

    for (hour_id, entries) in &hour_groups {
        let combined = entries.join("\n\n---\n\n");

        // Check for an existing hour node and merge content if present
        let (existing_summary, existing_created_at) =
            match store::read_node(config, namespace, hour_id)? {
                Some(existing) => (Some(existing.summary), Some(existing.created_at)),
                None => (None, None),
            };

        let to_summarize = if let Some(ref prev) = existing_summary {
            format!("{prev}\n\n---\n\n{combined}")
        } else {
            combined
        };

        let hour_summary = summarize_to_limit(
            provider,
            &to_summarize,
            NodeLevel::Hour.max_tokens(),
            "hour",
            hour_id,
        )
        .await
        .context("summarize hour leaf")?;

        let now = Utc::now();
        let hour_node = TreeNode {
            node_id: hour_id.clone(),
            namespace: namespace.to_string(),
            level: NodeLevel::Hour,
            parent_id: derive_parent_id(hour_id),
            summary: hour_summary.clone(),
            token_count: estimate_tokens(&hour_summary),
            child_count: 0,
            created_at: existing_created_at.unwrap_or(now),
            updated_at: now,
            metadata: None,
        };
        store::write_node(config, &hour_node)?;

        publish_global(DomainEvent::TreeSummarizerHourCompleted {
            namespace: namespace.to_string(),
            node_id: hour_id.clone(),
            token_count: hour_node.token_count,
        });

        tracing::debug!(
            "[tree_summarizer] hour leaf {} created ({} tokens)",
            hour_id,
            hour_node.token_count
        );

        // Derive propagation path for this hour
        let (_, day_id, month_id, year_id, root_id) = derive_node_ids_from_hour_id(hour_id);
        all_propagation_ids.push((day_id, NodeLevel::Day));
        all_propagation_ids.push((month_id, NodeLevel::Month));
        all_propagation_ids.push((year_id, NodeLevel::Year));
        all_propagation_ids.push((root_id, NodeLevel::Root));

        last_hour_node = Some(hour_node);
    }

    // Deduplicate and propagate in bottom-up order (days, months, years, root)
    let mut seen = std::collections::HashSet::new();
    for level in [
        NodeLevel::Day,
        NodeLevel::Month,
        NodeLevel::Year,
        NodeLevel::Root,
    ] {
        for (node_id, node_level) in &all_propagation_ids {
            if *node_level == level && seen.insert(node_id.clone()) {
                propagate_node(config, provider, namespace, node_id, level)
                    .await
                    .with_context(|| format!("propagate {node_id}"))?;
            }
        }
    }

    // All hour leaves are durably written and propagation is complete.
    // Now it's safe to delete the buffer entries.
    store::buffer_delete(config, namespace, &buffer_filenames)
        .context("delete buffer entries after successful summarization")?;

    Ok(last_hour_node)
}

/// Rebuild the entire tree from hour leaves upward.
/// Deletes all non-leaf nodes and re-summarizes.
/// Preserves buffered data that hasn't been summarized yet.
pub async fn rebuild_tree(
    config: &Config,
    provider: &dyn Provider,
    namespace: &str,
) -> Result<TreeStatus> {
    tracing::debug!("[tree_summarizer] rebuilding tree for namespace '{namespace}'");

    let status = store::get_tree_status(config, namespace)?;
    if status.total_nodes == 0 {
        return Ok(status);
    }

    // Collect all hour leaves first
    let base = store::tree_dir(config, namespace);
    let mut hour_leaves: Vec<TreeNode> = Vec::new();
    collect_hour_leaves_recursive(&base, namespace, "", &mut hour_leaves)?;

    if hour_leaves.is_empty() {
        tracing::debug!("[tree_summarizer] no hour leaves found, nothing to rebuild");
        return store::get_tree_status(config, namespace);
    }

    // Preserve the buffer directory by moving it to a sibling path *outside*
    // the tree directory, so delete_tree() does not destroy it.
    let buffer_path = store::buffer_dir(config, namespace);
    let tree_base = store::tree_dir(config, namespace);
    // Place backup next to the tree dir (e.g. .../tree_buffer_backup)
    let buffer_backup = tree_base
        .parent()
        .unwrap_or(&tree_base)
        .join("tree_buffer_backup");
    let buffer_existed = buffer_path.exists();
    if buffer_existed {
        if buffer_backup.exists() {
            std::fs::remove_dir_all(&buffer_backup)?;
        }
        std::fs::rename(&buffer_path, &buffer_backup).context("backup buffer before rebuild")?;
        tracing::debug!("[tree_summarizer] backed up buffer directory outside tree");
    }

    // Delete and recreate the tree directory
    store::delete_tree(config, namespace)?;

    // Restore the buffer directory back inside the tree
    if buffer_existed && buffer_backup.exists() {
        let restored_buffer = store::buffer_dir(config, namespace);
        if let Some(parent) = restored_buffer.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&buffer_backup, &restored_buffer)
            .context("restore buffer after rebuild")?;
        tracing::debug!("[tree_summarizer] restored buffer directory");
    }

    // Re-write all hour leaves
    for leaf in &hour_leaves {
        store::write_node(config, leaf)?;
    }

    // Collect unique ancestor IDs at each level, ordered bottom-up
    let mut day_ids = std::collections::BTreeSet::new();
    let mut month_ids = std::collections::BTreeSet::new();
    let mut year_ids = std::collections::BTreeSet::new();

    for leaf in &hour_leaves {
        if let Some(day) = derive_parent_id(&leaf.node_id) {
            day_ids.insert(day.clone());
            if let Some(month) = derive_parent_id(&day) {
                month_ids.insert(month.clone());
                if let Some(year) = derive_parent_id(&month) {
                    year_ids.insert(year);
                }
            }
        }
    }

    // Propagate bottom-up: days, then months, then years, then root
    for day_id in &day_ids {
        propagate_node(config, provider, namespace, day_id, NodeLevel::Day).await?;
    }
    for month_id in &month_ids {
        propagate_node(config, provider, namespace, month_id, NodeLevel::Month).await?;
    }
    for year_id in &year_ids {
        propagate_node(config, provider, namespace, year_id, NodeLevel::Year).await?;
    }
    propagate_node(config, provider, namespace, "root", NodeLevel::Root).await?;

    let final_status = store::get_tree_status(config, namespace)?;

    publish_global(DomainEvent::TreeSummarizerRebuildCompleted {
        namespace: namespace.to_string(),
        total_nodes: final_status.total_nodes,
    });

    tracing::debug!(
        "[tree_summarizer] rebuild complete for '{}': {} nodes",
        namespace,
        final_status.total_nodes
    );
    Ok(final_status)
}

// ── Internal ───────────────────────────────────────────────────────────

/// Re-summarize a single non-leaf node from its children.
async fn propagate_node(
    config: &Config,
    provider: &dyn Provider,
    namespace: &str,
    node_id: &str,
    level: NodeLevel,
) -> Result<()> {
    let children = store::read_children(config, namespace, node_id)?;
    if children.is_empty() {
        tracing::debug!(
            "[tree_summarizer] node {} has no children, skipping propagation",
            node_id
        );
        return Ok(());
    }

    let child_count = children.len() as u32;
    let combined: String = children
        .iter()
        .map(|c| format!("## {} ({})\n\n{}", c.node_id, c.level.as_str(), c.summary))
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");

    let combined_tokens = estimate_tokens(&combined);
    let max_tokens = level.max_tokens();

    let summary = if combined_tokens <= max_tokens {
        // Fits within budget — use the combined text directly
        tracing::debug!(
            "[tree_summarizer] node {} combined children ({} tokens) fits within {} token budget, no LLM needed",
            node_id,
            combined_tokens,
            max_tokens
        );
        combined
    } else {
        // Exceeds budget — summarize with LLM
        tracing::debug!(
            "[tree_summarizer] node {} combined children ({} tokens) exceeds {} token budget, summarizing",
            node_id,
            combined_tokens,
            max_tokens
        );
        summarize_to_limit(provider, &combined, max_tokens, level.as_str(), node_id).await?
    };

    let now = Utc::now();
    let existing = store::read_node(config, namespace, node_id)?;
    let created_at = existing.map(|n| n.created_at).unwrap_or(now);

    let node = TreeNode {
        node_id: node_id.to_string(),
        namespace: namespace.to_string(),
        level,
        parent_id: derive_parent_id(node_id),
        summary: summary.clone(),
        token_count: estimate_tokens(&summary),
        child_count,
        created_at,
        updated_at: now,
        metadata: None,
    };
    store::write_node(config, &node)?;

    publish_global(DomainEvent::TreeSummarizerPropagated {
        namespace: namespace.to_string(),
        node_id: node_id.to_string(),
        level: level.as_str().to_string(),
        token_count: node.token_count,
    });

    tracing::debug!(
        "[tree_summarizer] propagated node {} (level={}, tokens={}, children={})",
        node_id,
        level.as_str(),
        node.token_count,
        child_count
    );
    Ok(())
}

/// Summarize text to fit within a token limit using the LLM provider.
/// Enforces a hard character limit on the response to prevent runaway output.
async fn summarize_to_limit(
    provider: &dyn Provider,
    content: &str,
    max_tokens: u32,
    level_name: &str,
    node_id: &str,
) -> Result<String> {
    let max_chars = (max_tokens as usize) * 4;
    let system_prompt = format!(
        "You are a hierarchical summarizer. Compress the following content into a concise \
         summary that preserves the most important information.\n\n\
         Rules:\n\
         - The summary MUST be under {max_tokens} tokens (roughly {max_chars} characters).\n\
         - Focus on key events, decisions, facts, patterns, and actionable insights.\n\
         - Preserve names, dates, numbers, and specific details when important.\n\
         - Use clear, dense prose — no filler.\n\n\
         Context: You are summarizing at the {level_name} level for node '{node_id}'.",
    );

    let response = provider
        .chat_with_system(
            Some(&system_prompt),
            content,
            SUMMARIZATION_MODEL,
            SUMMARIZATION_TEMP,
        )
        .await
        .with_context(|| {
            format!("LLM summarization failed for node {node_id} (level={level_name})")
        })?;

    // Enforce hard character limit on LLM response (use the stricter of the two limits)
    let char_limit = max_chars.min(MAX_SUMMARY_CHARS);
    let response = if response.len() > char_limit {
        tracing::warn!(
            "[tree_summarizer] LLM response for node {} (level={}) was {} chars, truncating to {} chars",
            node_id,
            level_name,
            response.len(),
            char_limit
        );
        // Truncate at a char boundary
        let truncated = &response[..response.floor_char_boundary(char_limit)];
        truncated.to_string()
    } else {
        response
    };

    tracing::debug!(
        "[tree_summarizer] LLM summarized {} chars -> {} chars for node {} (level={})",
        content.len(),
        response.len(),
        node_id,
        level_name
    );

    Ok(response)
}

/// Group buffer entries by their hour based on filename timestamps.
///
/// Buffer filenames are `{timestamp_millis}_{uuid}.md`. We extract the timestamp
/// and derive the hour ID for each entry.
fn group_by_hour(entries: &[(String, String)]) -> BTreeMap<String, Vec<String>> {
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for (filename, content) in entries {
        let hour_id = hour_id_from_buffer_filename(filename).unwrap_or_else(|| {
            // Fallback: use current time if filename can't be parsed
            let now = Utc::now();
            let (hour, _, _, _, _) = derive_node_ids(&now);
            hour
        });
        groups.entry(hour_id).or_default().push(content.clone());
    }

    groups
}

/// Extract the hour node ID from a buffer filename like `1711972800000_abc12345.md`.
fn hour_id_from_buffer_filename(filename: &str) -> Option<String> {
    let ts_str = filename.split('_').next()?;
    let millis: i64 = ts_str.parse().ok()?;
    let dt = DateTime::from_timestamp_millis(millis)?;
    let (hour_id, _, _, _, _) = derive_node_ids(&dt);
    Some(hour_id)
}

/// Derive propagation IDs from an hour node_id string like "2024/03/15/14".
fn derive_node_ids_from_hour_id(hour_id: &str) -> (String, String, String, String, String) {
    let parts: Vec<&str> = hour_id.split('/').collect();
    if parts.len() == 4 {
        let year = parts[0].to_string();
        let month = format!("{}/{}", parts[0], parts[1]);
        let day = format!("{}/{}/{}", parts[0], parts[1], parts[2]);
        (hour_id.to_string(), day, month, year, "root".to_string())
    } else {
        // Fallback
        (
            hour_id.to_string(),
            "unknown".to_string(),
            "unknown".to_string(),
            "unknown".to_string(),
            "root".to_string(),
        )
    }
}

/// Recursively collect all hour leaf nodes from the tree directory.
fn collect_hour_leaves_recursive(
    dir: &std::path::Path,
    namespace: &str,
    prefix: &str,
    leaves: &mut Vec<TreeNode>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        let ft = entry.file_type()?;

        if ft.is_dir() {
            if name == "buffer" || name == "buffer_backup" {
                continue;
            }
            let child_prefix = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            collect_hour_leaves_recursive(&entry.path(), namespace, &child_prefix, leaves)?;
        } else if ft.is_file() && name.ends_with(".md") && name != "summary.md" && name != "root.md"
        {
            let hour_part = name.trim_end_matches(".md");
            let node_id = if prefix.is_empty() {
                hour_part.to_string()
            } else {
                format!("{prefix}/{hour_part}")
            };
            let level = level_from_node_id(&node_id);
            if level == NodeLevel::Hour {
                let raw = std::fs::read_to_string(entry.path())?;
                let node = crate::openhuman::tree_summarizer::store::parse_node_markdown_pub(
                    &raw, namespace, &node_id,
                )
                .with_context(|| format!("failed to parse hour leaf '{node_id}'"))?;
                leaves.push(node);
            }
        }
    }
    Ok(())
}

// ── Hourly background loop ─────────────────────────────────────────────

/// Start a background task that runs the summarization job every hour.
///
/// This should be called once at application startup. The task runs
/// indefinitely, sleeping until the next hour boundary.
pub async fn run_hourly_loop(config: Config, provider: Box<dyn Provider>) {
    tracing::debug!("[tree_summarizer] hourly loop started");

    loop {
        // Sleep until the next hour boundary
        let now = Utc::now();
        let next_hour = {
            let base = now
                .date_naive()
                .and_hms_opt(now.hour(), 0, 0)
                .unwrap_or(now.naive_utc());
            let next = base + chrono::Duration::hours(1);
            DateTime::<Utc>::from_naive_utc_and_offset(next, Utc)
        };
        let sleep_duration = (next_hour - now)
            .to_std()
            .unwrap_or(std::time::Duration::from_secs(3600));

        tracing::debug!(
            "[tree_summarizer] sleeping {:.0}s until next hour boundary",
            sleep_duration.as_secs_f64()
        );
        tokio::time::sleep(sleep_duration).await;

        // Run summarization for all namespaces that have buffered data
        let ts = Utc::now();
        let namespaces = discover_active_namespaces(&config);
        for ns in &namespaces {
            match run_summarization(&config, provider.as_ref(), ns, ts).await {
                Ok(Some(node)) => {
                    tracing::debug!(
                        "[tree_summarizer] hourly job completed for '{}': node {} ({} tokens)",
                        ns,
                        node.node_id,
                        node.token_count
                    );
                }
                Ok(None) => {
                    tracing::debug!(
                        "[tree_summarizer] hourly job skipped for '{}' (no buffered data)",
                        ns
                    );
                }
                Err(e) => {
                    tracing::error!("[tree_summarizer] hourly job failed for '{}': {:#}", ns, e);
                }
            }
        }
    }
}

/// Discover namespaces that have pending buffer data by scanning the
/// `memory/namespaces/*/tree/buffer/` directories.
fn discover_active_namespaces(config: &Config) -> Vec<String> {
    let namespaces_dir = config.workspace_dir.join("memory").join("namespaces");

    if !namespaces_dir.exists() {
        return vec![];
    }

    let mut active = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&namespaces_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let buffer_dir = entry.path().join("tree").join("buffer");
            if buffer_dir.exists() {
                // Check if buffer has any .md files
                if let Ok(buffer_entries) = std::fs::read_dir(&buffer_dir) {
                    let has_entries = buffer_entries
                        .flatten()
                        .any(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false));
                    if has_entries {
                        active.push(name);
                    }
                }
            }
        }
    }
    active
}
