//! Assembles a bounded "situation report" for the subconscious tick.
//! Gathers deltas since the last tick from memory, tools, environment,
//! and HEARTBEAT.md — capped at the configured token budget.

use crate::openhuman::memory::MemoryClient;
use std::fmt::Write;
use std::path::Path;

/// Rough chars-per-token estimate for budget enforcement.
const CHARS_PER_TOKEN: usize = 4;

/// Assemble the situation report for a subconscious tick.
///
/// `last_tick_at` is 0.0 on the first tick (cold start → include everything).
/// `token_budget` caps the output size; sections are truncated if exceeded.
pub async fn build_situation_report(
    memory: Option<&MemoryClient>,
    workspace_dir: &Path,
    last_tick_at: f64,
    token_budget: u32,
) -> String {
    let char_budget = (token_budget as usize) * CHARS_PER_TOKEN;
    let mut report = String::with_capacity(char_budget.min(64_000));
    let mut remaining = char_budget;

    // Section 1: Environment
    let env_section = build_environment_section(workspace_dir);
    append_section(&mut report, &mut remaining, &env_section);

    // Section 2: HEARTBEAT.md tasks
    let tasks_section = build_tasks_section(workspace_dir).await;
    append_section(&mut report, &mut remaining, &tasks_section);

    // Section 3: Memory documents (delta since last tick)
    if let Some(client) = memory {
        let docs_section = build_memory_docs_section(client, last_tick_at).await;
        append_section(&mut report, &mut remaining, &docs_section);

        // Section 4: Graph relations (delta since last tick)
        let graph_section = build_graph_section(client, last_tick_at).await;
        append_section(&mut report, &mut remaining, &graph_section);
    } else {
        append_section(
            &mut report,
            &mut remaining,
            "## Memory\n\nMemory client not available.\n",
        );
    }

    // Section 5: Skills runtime health
    let skills_section = build_skills_section().await;
    append_section(&mut report, &mut remaining, &skills_section);

    if report.trim().is_empty() {
        report.push_str("No state changes detected since last tick.\n");
    }

    report
}

fn build_environment_section(workspace_dir: &Path) -> String {
    let host =
        hostname::get().map_or_else(|_| "unknown".into(), |h| h.to_string_lossy().to_string());
    let now = chrono::Local::now();
    format!(
        "## Environment\n\n\
         Workspace: {}\n\
         Host: {} | OS: {}\n\
         Time: {}\n",
        workspace_dir.display(),
        host,
        std::env::consts::OS,
        now.format("%Y-%m-%d %H:%M:%S %Z"),
    )
}

async fn build_tasks_section(workspace_dir: &Path) -> String {
    let heartbeat_path = workspace_dir.join("HEARTBEAT.md");
    let content = match tokio::fs::read_to_string(&heartbeat_path).await {
        Ok(c) => c,
        Err(_) => return "## Pending Tasks\n\nNo HEARTBEAT.md found.\n".to_string(),
    };

    let tasks: Vec<&str> = content
        .lines()
        .filter_map(|line| line.trim().strip_prefix("- "))
        .collect();

    if tasks.is_empty() {
        return "## Pending Tasks\n\nNo tasks defined.\n".to_string();
    }

    let mut section = String::from("## Pending Tasks\n\n");
    for task in &tasks {
        let _ = writeln!(section, "- {task}");
    }
    section
}

async fn build_memory_docs_section(client: &MemoryClient, last_tick_at: f64) -> String {
    let docs = match client.list_documents(None).await {
        Ok(raw) => raw,
        Err(e) => {
            return format!("## Memory Documents\n\nFailed to list documents: {e}\n");
        }
    };

    // Parse the raw serde_json::Value into document summaries
    let doc_array = docs
        .as_array()
        .or_else(|| docs.get("documents").and_then(|v| v.as_array()));

    let Some(doc_array) = doc_array else {
        return "## Memory Documents\n\nNo documents found.\n".to_string();
    };

    // Filter to docs updated since last tick (or all on cold start)
    let is_cold_start = last_tick_at <= 0.0;
    let mut new_docs: Vec<(&str, &str, f64)> = Vec::new();
    for doc in doc_array {
        let updated_at = doc
            .get("updated_at")
            .or_else(|| doc.get("updatedAt"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        if !is_cold_start && updated_at <= last_tick_at {
            continue;
        }

        let title = doc
            .get("title")
            .or_else(|| doc.get("key"))
            .and_then(|v| v.as_str())
            .unwrap_or("untitled");
        let namespace = doc.get("namespace").and_then(|v| v.as_str()).unwrap_or("?");

        new_docs.push((namespace, title, updated_at));
    }

    if new_docs.is_empty() {
        return format!(
            "## Memory Documents\n\n{} total documents. No changes since last tick.\n",
            doc_array.len()
        );
    }

    let mut section = format!(
        "## Memory Documents\n\n{} total, {} new/updated since last tick:\n\n",
        doc_array.len(),
        new_docs.len()
    );
    for (namespace, title, _) in &new_docs {
        let _ = writeln!(section, "- [{namespace}] {title}");
    }
    section
}

async fn build_graph_section(client: &MemoryClient, last_tick_at: f64) -> String {
    let relations = match client.graph_query(None, None, None).await {
        Ok(rows) => rows,
        Err(e) => {
            return format!("## Knowledge Graph\n\nFailed to query graph: {e}\n");
        }
    };

    if relations.is_empty() {
        return "## Knowledge Graph\n\nNo relations.\n".to_string();
    }

    // Filter to relations updated since last tick
    let is_cold_start = last_tick_at <= 0.0;
    let mut new_relations: Vec<(&str, &str, &str)> = Vec::new();
    for rel in &relations {
        let updated_at = rel
            .get("updatedAt")
            .or_else(|| rel.get("updated_at"))
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        if !is_cold_start && updated_at <= last_tick_at {
            continue;
        }

        let subject = rel.get("subject").and_then(|v| v.as_str()).unwrap_or("?");
        let predicate = rel.get("predicate").and_then(|v| v.as_str()).unwrap_or("?");
        let object = rel.get("object").and_then(|v| v.as_str()).unwrap_or("?");
        new_relations.push((subject, predicate, object));
    }

    if new_relations.is_empty() {
        return format!(
            "## Knowledge Graph\n\n{} total relations. No changes since last tick.\n",
            relations.len()
        );
    }

    let mut section = format!(
        "## Knowledge Graph\n\n{} total, {} new/updated:\n\n",
        relations.len(),
        new_relations.len()
    );
    for (s, p, o) in new_relations.iter().take(20) {
        let _ = writeln!(section, "- {s} → {p} → {o}");
    }
    if new_relations.len() > 20 {
        let _ = writeln!(section, "- ... and {} more", new_relations.len() - 20);
    }
    section
}

async fn build_skills_section() -> String {
    // Call the skills_list RPC internally via the controller registry
    let params = serde_json::Map::new();
    let result = crate::core::all::try_invoke_registered_rpc("openhuman.skills_list", params).await;

    let skills_value = match result {
        Some(Ok(value)) => value,
        Some(Err(_)) | None => {
            return "## Skills Runtime\n\nSkill registry unavailable.\n".to_string();
        }
    };

    let skills = match skills_value.as_array() {
        Some(arr) => arr,
        None => {
            return "## Skills Runtime\n\nNo skills data.\n".to_string();
        }
    };

    if skills.is_empty() {
        return "## Skills Runtime\n\nNo skills installed.\n".to_string();
    }

    let mut section = String::from("## Skills Runtime\n\n");
    for skill in skills {
        let name = skill
            .get("skill_id")
            .or_else(|| skill.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let status = skill
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let setup = skill
            .get("setup_complete")
            .and_then(|v| v.as_bool())
            .map(|b| if b { "ready" } else { "not setup" })
            .unwrap_or("?");
        let connection = skill
            .get("connection_status")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let mut line = format!("- {name}: {status}");
        if !connection.is_empty() && connection != status {
            line.push_str(&format!(" ({connection})"));
        }
        if setup == "not setup" {
            line.push_str(" [not setup]");
        }

        if let Some(err) = skill
            .get("connection_error")
            .or_else(|| skill.get("lastError"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            line.push_str(&format!(" ERROR: {}", &err[..err.len().min(100)]));
        }

        let _ = writeln!(section, "{line}");
    }
    section
}

fn append_section(report: &mut String, remaining: &mut usize, section: &str) {
    if *remaining == 0 {
        return;
    }
    // +1 for the trailing newline we append
    let needed = section.len().saturating_add(1);
    if needed <= *remaining {
        report.push_str(section);
        report.push('\n');
        *remaining -= needed;
    } else {
        // Truncate at a valid UTF-8 char boundary
        let budget = *remaining;
        let truncate_at = section
            .char_indices()
            .take_while(|(i, _)| *i < budget)
            .last()
            .map(|(i, ch)| i + ch.len_utf8())
            .unwrap_or(0);
        report.push_str(&section[..truncate_at]);
        report.push_str("\n[... truncated — token budget exceeded]\n");
        *remaining = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_section_contains_os_and_host() {
        let section = build_environment_section(Path::new("/tmp/workspace"));
        assert!(section.contains("## Environment"));
        assert!(section.contains("Workspace: /tmp/workspace"));
        assert!(section.contains("OS:"));
    }

    #[test]
    fn append_section_truncates_on_budget() {
        let mut report = String::new();
        let mut remaining = 10;
        append_section(&mut report, &mut remaining, "Hello, this is a long section");
        assert!(report.starts_with("Hello, thi"));
        assert!(report.contains("truncated"));
        assert_eq!(remaining, 0);
    }

    #[test]
    fn append_section_exact_fit_does_not_underflow() {
        let mut report = String::new();
        // "Hello" (5 bytes) + newline (1 byte) = 6 bytes needed
        let mut remaining = 6;
        append_section(&mut report, &mut remaining, "Hello");
        assert_eq!(report, "Hello\n");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn append_section_truncates_at_char_boundary() {
        let mut report = String::new();
        // "日本語" is 9 bytes (3 chars × 3 bytes each)
        // Budget of 5 should truncate to "日" (3 bytes), not panic
        let mut remaining = 5;
        append_section(&mut report, &mut remaining, "日本語タスク");
        assert!(report.starts_with("日"));
        assert!(report.contains("truncated"));
        assert_eq!(remaining, 0);
    }

    #[test]
    fn append_section_fits_within_budget() {
        let mut report = String::new();
        let mut remaining = 1000;
        append_section(&mut report, &mut remaining, "Short");
        assert!(report.contains("Short"));
        assert!(remaining < 1000);
    }
}
