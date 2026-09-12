//! RPC handler implementations for memory sources.
//!
//! # How this file reaches memory (#5560)
//!
//! Every memory call below goes through the bound driver — never through an
//! in-process engine handle. The four surfaces this file used to reach into
//! `tinymemory_core::tinycortex` for became contract members in tinymemory
//! v1.7.0, and the mapping is one-for-one:
//!
//! | What a handler needs | Contract member |
//! |---|---|
//! | coding-session discovery | `MemoryCodingSessions::coding_session_status` |
//! | coding-session ingestion | `MemoryCodingSessions::ingest_coding_sessions` |
//! | raw-archive coverage, and its repair | `MemorySourceSync::raw_archive_coverage` / `rebuild_from_raw_archive` |
//! | the sync audit log | `MemorySourceSync::sync_audit_log` |
//! | the sync price | `MemorySourceSync::estimate_sync_cost_usd` |
//!
//! Two of those look shortcuttable and are not, so the reasons are recorded
//! here rather than left to be re-derived:
//!
//! - **The audit log is not read through the `memory::sync::audit` host shim.**
//!   That path resolves, and taking it would move this file off the
//!   direct-engine-reference scanner while removing no coupling whatsoever —
//!   precisely the edit the ratchet's own docs call the one that would make the
//!   lint lie.
//! - **The price is asked, never computed here.** The same constants behind
//!   `estimate_sync_cost_usd` stamped `estimated_cost_usd` onto every audit row
//!   the driver has already written, and `monthly_cost_summary_rpc` below totals
//!   those very rows. A host-side copy of the arithmetic becomes a second price
//!   the moment either side is retuned, and one screen would then show a
//!   projection and a history priced differently with nothing to say which.
//!
//! ## Refusing when the driver does not serve the family
//!
//! `as_source_sync()` / `as_coding_sessions()` answering `None` means the bound
//! driver serves no such family, and every handler here **refuses, naming the
//! driver** (see [`unserved`]). None of them reports an empty log, a zero cost
//! or an empty source list instead.
//!
//! That is a deliberately different trade from the tree read handlers, which do
//! degrade to empty. There, "no hits" is a true statement about a driver that
//! keeps no summary tree. Here the family *is* the entire subject of the call,
//! so an empty success is indistinguishable from "nothing has ever synced" or
//! "no coding agent is installed" — a wrong answer the caller cannot tell from
//! a right one, on the two screens where the number is a promise about money
//! and about how long an import will take.
//!
//! ## On this file's length
//!
//! It is over the ~500-line guidance, and was before this change; it is not
//! split as part of it. The seam is visible, though, and the routing made it
//! sharper: `coding_session_status_rpc`, `ingest_coding_sessions_rpc`,
//! `reconcile_rpc`, `sync_audit_log_rpc`, `estimate_sync_cost_rpc` and
//! `monthly_cost_summary_rpc` are the driver-facing half — six handlers over
//! two capability families, all sharing the binding preamble — while the
//! registry CRUD between them talks only to `registry` and `readers` and never
//! resolves a binding at all.

#[cfg(test)]
#[path = "rpc_filter_tests_tests.rs"]
mod filter_tests;

#[cfg(test)]
#[path = "rpc_supported_toolkits_tests_tests.rs"]
mod supported_toolkits_tests;

#[cfg(test)]
#[path = "rpc_budget_tests_tests.rs"]
mod budget_tests;

#[cfg(test)]
#[path = "rpc_monthly_summary_tests_tests.rs"]
mod monthly_summary_tests;

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod rpc_tests;
include!("rpc_part_01.rs");
include!("rpc_part_02.rs");
