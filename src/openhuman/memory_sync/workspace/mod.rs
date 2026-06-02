//! Workspace-scoped sync pipelines.
//!
//! Pipelines that pull from sources local to the user's workspace rather
//! than third-party services. Three flavors expected:
//!
//! | Submodule | Source | Notes |
//! | --- | --- | --- |
//! | `folder`    | Files under a user-added folder memory source          | Watch + diff |
//! | `harness`   | Agent harness turns (memory_archivist's caller side)   | Push-based |
//! | `dictation` | Local audio capture transcripts                        | Push-based |
//!
//! ## Status
//!
//! Scaffold only. Today folder ingestion lives in
//! `memory_sources/readers/folder.rs`, harness capture in
//! `agent_experience/`, and dictation in `dictation_hotkeys/`. Each will
//! land here as a [`SyncPipeline`] impl in a follow-up.
