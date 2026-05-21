//! Curated catalog of ClickUp Composio actions exposed to the agent.
//!
//! Slugs match Composio's naming convention (`<TOOLKIT>_<ACTION>`) for
//! the ClickUp REST surface. See <https://composio.dev/docs/toolkits/clickup>
//! for the canonical action list; the entries here are the read-oriented
//! subset the periodic Memory Tree sync relies on, plus the most common
//! task-write surface the agent already uses through generic tool-calling.

use crate::openhuman::composio::providers::tool_scope::{CuratedTool, ToolScope};

pub const CLICKUP_CURATED: &[CuratedTool] = &[
    // ── Read: identity ─────────────────────────────────────────────
    CuratedTool {
        slug: "CLICKUP_GET_AUTHORIZED_USER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "CLICKUP_GET_AUTHORIZED_TEAMS_WORKSPACES",
        scope: ToolScope::Read,
    },
    // ── Read: structure (workspace → space → folder → list) ──────
    CuratedTool {
        slug: "CLICKUP_GET_SPACES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "CLICKUP_GET_FOLDERS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "CLICKUP_GET_LISTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "CLICKUP_GET_FOLDERLESS_LISTS",
        scope: ToolScope::Read,
    },
    // ── Read: tasks (the main memory ingest surface) ──────────────
    CuratedTool {
        slug: "CLICKUP_GET_FILTERED_TEAM_TASKS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "CLICKUP_GET_TASKS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "CLICKUP_GET_TASK",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "CLICKUP_GET_TASK_COMMENTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "CLICKUP_GET_LIST_COMMENTS",
        scope: ToolScope::Read,
    },
    // ── Read: docs / views / time tracking ────────────────────────
    CuratedTool {
        slug: "CLICKUP_SEARCH_DOCS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "CLICKUP_GET_DOC_PAGES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "CLICKUP_GET_VIEW_TASKS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "CLICKUP_GET_TIME_ENTRIES_WITHIN_A_DATE_RANGE",
        scope: ToolScope::Read,
    },
    // ── Read: members ─────────────────────────────────────────────
    CuratedTool {
        slug: "CLICKUP_GET_WORKSPACE_MEMBERS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "CLICKUP_GET_TASK_MEMBERS",
        scope: ToolScope::Read,
    },
    // ── Write: create / update tasks ──────────────────────────────
    CuratedTool {
        slug: "CLICKUP_CREATE_TASK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "CLICKUP_UPDATE_TASK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "CLICKUP_CREATE_TASK_COMMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "CLICKUP_UPDATE_COMMENT",
        scope: ToolScope::Write,
    },
    // ── Write: structure ──────────────────────────────────────────
    CuratedTool {
        slug: "CLICKUP_CREATE_LIST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "CLICKUP_UPDATE_LIST",
        scope: ToolScope::Write,
    },
    // ── Admin: destructive ────────────────────────────────────────
    CuratedTool {
        slug: "CLICKUP_DELETE_TASK",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "CLICKUP_DELETE_COMMENT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "CLICKUP_DELETE_LIST",
        scope: ToolScope::Admin,
    },
];
