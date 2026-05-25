//! Workspace-scoped sync pipelines.
//!
//! Pipelines that pull from sources local to the user's workspace rather
//! than third-party services. Three flavors expected:
//!
//! | Submodule | Source | Notes |
//! | --- | --- | --- |
//! | `vault`     | Files dropped into the Obsidian vault by the user      | Watch + diff |
//! | `harness`   | Agent harness turns (memory_archivist's caller side)   | Push-based |
//! | `dictation` | Local audio capture transcripts                        | Push-based |
//!
//! ## Status
//!
//! Scaffold only. Today the vault watch lives in `vault/sync.rs`,
//! harness capture in `agent_experience/`, and dictation in
//! `dictation_hotkeys/`. Each will land here as a [`SyncPipeline`] impl
//! in a follow-up.
