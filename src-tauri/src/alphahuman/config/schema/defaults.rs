//! Shared default value helpers used by multiple config structs.

/// Used by gateway, tools, storage/memory, autonomy, runtime for serde defaults.
pub fn default_true() -> bool {
    true
}
