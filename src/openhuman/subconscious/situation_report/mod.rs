//! Situation report assembly for the subconscious tick (#623).
//!
//! Replaces the legacy unified-store-backed report with sections derived
//! from the memory tree:
//!
//! 1. **Environment** (kept): host/OS/workspace/time anchor.
//! 2. **Your Identifiers** (#1365): the user's connected-account
//!    identifiers (Slack/Gmail/Notion handles, emails, user_ids) so the
//!    reflection LLM can disambiguate body-text mentions — "Cyrus said X"
//!    is the user iff `Cyrus` (or the email/handle) appears in this list.
//! 3. **Pending Tasks** (kept): subconscious task list from SQLite.
//! 4. **Hotness deltas** (new): top movers in `mem_tree_entity_hotness`
//!    since the last tick. Highest signal density. Items tagged `(you)`
//!    are the user's own identifiers (#1365).
//! 5. **Recently-sealed summaries** (new): rows from `mem_tree_summaries`
//!    grouped by tree.
//! 6. **Latest global L0 digest** (new): most recent daily digest body.
//! 7. **`query_global` recap window** (new): since `last_tick_at`.
//! 8. **Recent reflections** (new): the last N reflections from the
//!    subconscious store, used by the LLM as anti-double-emit context.
//!
//! Sections are appended in priority order; truncation drops the tail
//! when `token_budget` is exceeded. The legacy unified-store sections
//! (`MemoryClient::list_documents`, `graph_query`) and the local-skills
//! placeholder are intentionally dropped.
//!
//! Each submodule is responsible for one section so churn stays local.

use std::path::Path;

use crate::openhuman::config::Config;

use super::reflection::Reflection;

mod digest;
mod hotness;
mod query_window;
pub(crate) mod reflections;
mod summaries;

/// Rough chars-per-token estimate for budget enforcement.
const CHARS_PER_TOKEN: usize = 4;

/// Build the situation report for one subconscious tick.
///
/// `last_tick_at` is 0.0 on cold start (include everything in the
/// configured windows). `token_budget` caps total output; sections
/// after the cap are truncated with a marker.
///
/// Reflections come from `recent_reflections` so the caller can choose
/// whatever cursor logic suits them (typically: last 8 by `created_at`).
pub async fn build_situation_report(
    config: &Config,
    workspace_dir: &Path,
    last_tick_at: f64,
    token_budget: u32,
    recent_reflections: &[Reflection],
) -> String {
    let char_budget = (token_budget as usize) * CHARS_PER_TOKEN;
    let mut report = String::with_capacity(char_budget.min(64_000));
    let mut remaining = char_budget;

    // Section 1: environment anchor.
    let env_section = build_environment_section(workspace_dir);
    append_section(&mut report, &mut remaining, &env_section);

    // Section 2 (#1365): the user's connected-account identifiers, so
    // the reflection LLM can disambiguate "Cyrus said X" from body text
    // — that's the user iff the identifier list claims it.
    let identifiers_section = build_identifiers_section();
    append_section(&mut report, &mut remaining, &identifiers_section);

    // Section 3: pending subconscious tasks.
    let tasks_section = build_tasks_section(workspace_dir);
    append_section(&mut report, &mut remaining, &tasks_section);

    // Section 3: hotness deltas (highest priority memory-tree signal).
    let hotness_section = hotness::build_section(config, workspace_dir, last_tick_at).await;
    append_section(&mut report, &mut remaining, &hotness_section);

    // Section 4: recently-sealed summaries since last tick.
    let summaries_section = summaries::build_section(config, last_tick_at).await;
    append_section(&mut report, &mut remaining, &summaries_section);

    // Section 5: latest global L0 digest body — gated by `last_tick_at`
    // so a digest the previous tick already saw doesn't get re-fed and
    // re-cited (which was producing duplicate reflections).
    let digest_section = digest::build_section(config, last_tick_at).await;
    append_section(&mut report, &mut remaining, &digest_section);

    // Section 6: query_global recap window since last tick.
    let recap_section = query_window::build_section(config, last_tick_at).await;
    append_section(&mut report, &mut remaining, &recap_section);

    // Section 7: previous reflections (anti-double-emit context).
    let reflections_section = reflections::build_section(recent_reflections);
    append_section(&mut report, &mut remaining, &reflections_section);

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

/// Render the user's connected-account identifiers (#1365) so the
/// reflection LLM can correlate body-text mentions back to the user.
/// Empty string when no providers are connected — the section just
/// disappears rather than rendering an empty header.
fn build_identifiers_section() -> String {
    let identities = crate::openhuman::composio::providers::profile::load_connected_identities();
    if identities.is_empty() {
        return String::new();
    }
    let body = crate::openhuman::composio::providers::profile::render_connected_identities_section(
        &identities,
    );
    if body.trim().is_empty() {
        return String::new();
    }
    // The shared renderer emits "## Connected Identities". Rename the
    // heading for the situation-report context so the LLM knows this is
    // *the user's* identity surface, not a list of contacts.
    let renamed = body.replacen("## Connected Identities", "## Your Identifiers", 1);
    let mut out = renamed;
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(
        "\nWhen body text in later sections mentions any of the above (name, email, \
         handle, or user_id), treat it as the user's own activity. Anything else is \
         someone else.\n",
    );
    out
}

fn build_tasks_section(workspace_dir: &Path) -> String {
    use std::fmt::Write;
    let tasks = match super::store::with_connection(workspace_dir, |conn| {
        super::store::list_tasks(conn, false)
    }) {
        Ok(tasks) => tasks,
        Err(_) => return "## Pending Tasks\n\nFailed to read tasks.\n".to_string(),
    };

    if tasks.is_empty() {
        return "## Pending Tasks\n\nNo tasks defined.\n".to_string();
    }

    let mut section = String::from("## Pending Tasks\n\n");
    for task in &tasks {
        let _ = writeln!(section, "- {}", task.title);
    }
    section
}

/// Append a section, truncating at a UTF-8 char boundary if it overflows
/// the remaining budget. Once `remaining` hits zero, subsequent sections
/// are silently dropped (not even truncated marker added — caller
/// already noted the cap).
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
        let budget = *remaining;
        let truncate_at = crate::openhuman::util::floor_char_boundary(section, budget);
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
        let mut remaining = 6;
        append_section(&mut report, &mut remaining, "Hello");
        assert_eq!(report, "Hello\n");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn append_section_truncates_at_char_boundary() {
        let mut report = String::new();
        // "日本語" is 9 bytes (3 chars × 3 bytes each).
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
