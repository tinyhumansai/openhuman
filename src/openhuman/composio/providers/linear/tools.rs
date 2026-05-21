//! Curated catalog of Linear Composio actions exposed to the agent.
//!
//! Migrated from `catalogs_productivity::LINEAR_CURATED` so the
//! curated set now lives next to its native provider, matching the
//! `gmail` / `notion` / `clickup` layout (each provider owns its tool
//! catalog rather than reaching into `catalogs/*`).
//!
//! The previous `LINEAR_CURATED` constant in `catalogs_productivity.rs`
//! has been removed in this change — `catalog_for_toolkit("linear")`
//! now resolves through [`super::LINEAR_CURATED`].
//!
//! See <https://composio.dev/docs/toolkits/linear> for the canonical
//! action list. Adds `LINEAR_GET_VIEWER` (Linear's "current
//! authenticated user" probe) so the sync path can resolve the
//! connected user's id without an extra round-trip through teams.

use crate::openhuman::composio::providers::tool_scope::{CuratedTool, ToolScope};

pub const LINEAR_CURATED: &[CuratedTool] = &[
    // ── Read: identity ─────────────────────────────────────────────
    CuratedTool {
        slug: "LINEAR_GET_VIEWER",
        scope: ToolScope::Read,
    },
    // ── Read: issues (the main memory ingest surface) ─────────────
    CuratedTool {
        slug: "LINEAR_LIST_LINEAR_ISSUES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "LINEAR_GET_LINEAR_ISSUE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "LINEAR_SEARCH_ISSUES",
        scope: ToolScope::Read,
    },
    // ── Read: workspace structure ──────────────────────────────────
    CuratedTool {
        slug: "LINEAR_LIST_LINEAR_TEAMS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "LINEAR_LIST_LINEAR_PROJECTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "LINEAR_GET_LINEAR_PROJECT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "LINEAR_LIST_LINEAR_STATES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "LINEAR_GET_CYCLES_BY_TEAM_ID",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "LINEAR_LIST_LINEAR_USERS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "LINEAR_LIST_LINEAR_LABELS",
        scope: ToolScope::Read,
    },
    // ── Write: create issues / comments / projects ────────────────
    CuratedTool {
        slug: "LINEAR_CREATE_LINEAR_ISSUE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_UPDATE_ISSUE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_CREATE_LINEAR_COMMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_UPDATE_LINEAR_COMMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_CREATE_ATTACHMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_CREATE_LINEAR_PROJECT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_UPDATE_LINEAR_PROJECT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_CREATE_LINEAR_LABEL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_CREATE_ISSUE_RELATION",
        scope: ToolScope::Write,
    },
    // ── Admin: destructive ────────────────────────────────────────
    CuratedTool {
        slug: "LINEAR_DELETE_LINEAR_ISSUE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "LINEAR_REMOVE_ISSUE_LABEL",
        scope: ToolScope::Admin,
    },
];
