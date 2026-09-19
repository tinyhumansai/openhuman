//! Host-specific prompt construction, tool policy refresh, and the model graph.
//!
//! Generic turn lifecycle is intentionally absent: it is owned by
//! `tinyagents_runtime::Session` in `runtime_session`.

mod context;
pub(crate) mod graph;
mod tools;

/// Returns newly connected capability names and records the observation once.
pub(super) fn newly_connected_slugs(
    connected: &[String],
    announced: &mut std::collections::HashSet<String>,
) -> Vec<String> {
    let newly: Vec<String> = connected
        .iter()
        .filter(|slug| !announced.contains(*slug))
        .cloned()
        .collect();
    announced.extend(newly.iter().cloned());
    newly
}
