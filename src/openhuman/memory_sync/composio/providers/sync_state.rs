//! Compatibility exports for sync state now owned by tinycortex.

pub use tinycortex::memory::sync::state::DEFAULT_DAILY_REQUEST_LIMIT;
pub use tinycortex::memory::sync::{DailyBudget, SyncState};

pub const KV_NAMESPACE: &str = crate::openhuman::tinycortex::HOST_SYNC_STATE_NAMESPACE;

pub fn extract_item_id(item: &serde_json::Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| {
        let value = path
            .split('.')
            .try_fold(item, |current, segment| current.get(segment))?;
        value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}
