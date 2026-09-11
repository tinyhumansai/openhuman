//! Structure-only graph scaffold for `spawn_parallel_agents`.
//!
//! The tool wrapper still owns `ToolResult` translation. This module owns
//! request parsing, parent-context validation, graph-side request validation,
//! worktree preflight, progress/event projection, worker fanout, final JSON
//! formatting, and the topology surface (WP-5 of
//! `docs/tinyagents-migration-plan-2026-07-22.md`).
//!
//! **Write safety.** Whether a worker *needs* a claim on the shared workspace is
//! an OpenHuman decision — it reads sandbox mode, tool permissions and the
//! isolation request. Whether the claims of a whole batch can be granted
//! together is not, and goes through
//! [`plan_shared_workspace_dispatch`](tinyagents_graph::parallel::plan_shared_workspace_dispatch).
//! The rejection sentences stay here, which is why the crate reports conflicts
//! as data.

include!("spawn_parallel_graph_part_01.rs");
include!("spawn_parallel_graph_part_02.rs");
include!("spawn_parallel_graph_part_03.rs");
