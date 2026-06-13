//! Curated catalog of Linear Composio actions.

use crate::openhuman::memory_sync::composio::providers::tool_scope::{CuratedTool, ToolScope};

pub const LINEAR_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "LINEAR_LIST_LINEAR_USERS",
        scope: ToolScope::Read,
    },
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
        slug: "LINEAR_LIST_LINEAR_LABELS",
        scope: ToolScope::Read,
    },
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
        slug: "LINEAR_CREATE_ISSUE_RELATION",
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
        slug: "LINEAR_DELETE_LINEAR_ISSUE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "LINEAR_REMOVE_ISSUE_LABEL",
        scope: ToolScope::Admin,
    },
];
