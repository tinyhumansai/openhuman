//! Memory archivist — thin host shim over `tinycortex::memory::archivist` (W7).
//!
//! The archivist engine (clip / compose / tree-writer / episodic store + its
//! types) lives in the crate now. This module keeps the host's
//! `memory_archivist::…` import paths stable and adapts the host [`Config`] to
//! the crate's `MemoryConfig` for the two episodic-store functions host callers
//! use (`store::record_turn` / `store::session_entries`, from the agent
//! archivist hook + recap).
//!
//! On-disk layout is unchanged — the crate writes to the same path the host
//! did: `<workspace>/memory_tree/content/episodic/<sanitized-session>/<seq>.md`.
//! `Config::workspace_dir` maps to the crate `MemoryConfig.workspace`, so
//! `memory_tree_content_root()` (`<workspace>/memory_tree/content`) resolves
//! identically on both sides.

pub use tinycortex::memory::archivist::types::{ArchivedTurn, Turn};

/// Episodic conversation archive — thin adapters over the crate store that
/// convert the host [`Config`](crate::openhuman::config::Config) into the
/// crate's `MemoryConfig`.
pub mod store {
    use anyhow::Result;

    use super::ArchivedTurn;
    use crate::openhuman::config::Config;
    use crate::openhuman::tinycortex::memory_config_from;

    fn engine_config(config: &Config) -> tinycortex::memory::MemoryConfig {
        memory_config_from(config, config.workspace_dir.clone())
    }

    /// Append one turn to its session's episodic archive, assigning the next
    /// per-session sequence number.
    pub fn record_turn(config: &Config, turn: ArchivedTurn) -> Result<ArchivedTurn> {
        tinycortex::memory::archivist::store::record_turn(&engine_config(config), turn)
    }

    /// All turns recorded for a session, ordered by sequence.
    pub fn session_entries(config: &Config, session_id: &str) -> Result<Vec<ArchivedTurn>> {
        tinycortex::memory::archivist::store::session_entries(&engine_config(config), session_id)
    }
}

pub use store::{record_turn, session_entries};
