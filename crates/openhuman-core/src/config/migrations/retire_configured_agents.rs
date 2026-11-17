//! Drop the retired top-level `[agents]` configuration before typed decoding.
//!
//! Older versions used this table to configure synchronous provider-backed
//! delegate tools. The host no longer supports that surface, so preserve all
//! other configuration while ignoring this stale table on load.

/// Remove the legacy configured-agent table from a parsed TOML document.
pub(crate) fn rewrite_document(document: &mut toml::Value) {
    if let Some(table) = document.as_table_mut() {
        table.remove("agents");
    }
}

#[cfg(test)]
#[path = "retire_configured_agents_tests.rs"]
mod tests;
