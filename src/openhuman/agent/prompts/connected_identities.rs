//! Connected identity prompt helper.
//!
//! Kept in a dedicated sibling module so `mod.rs` remains mostly
//! export-focused while the runtime fetch logic lives in a small,
//! testable unit.

/// Render persisted provider identities (if available) as a compact
/// `## Connected Identities` section.
pub fn render_connected_identities() -> String {
    let identities = crate::openhuman::composio::providers::profile::load_connected_identities();
    crate::openhuman::composio::providers::profile::render_connected_identities_section(&identities)
}
