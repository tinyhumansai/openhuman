//! Curated catalog of Notion Composio actions exposed to the agent.

use crate::openhuman::composio::providers::tool_scope::{CuratedTool, ToolScope};

pub const NOTION_CURATED: &[CuratedTool] = &[
    // ── Read: search & fetch ────────────────────────────────────────
    CuratedTool { slug: "NOTION_SEARCH_NOTION_PAGE", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_FETCH_DATA", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_FETCH_DATABASE", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_FETCH_ROW", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_FETCH_BLOCK_METADATA", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_FETCH_BLOCK_CONTENTS", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_FETCH_ALL_BLOCK_CONTENTS", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_FETCH_COMMENTS", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_GET_PAGE_MARKDOWN", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_GET_PAGE_PROPERTY_ACTION", scope: ToolScope::Read },
    // ── Read: query & retrieve ──────────────────────────────────────
    CuratedTool { slug: "NOTION_QUERY_DATABASE", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_QUERY_DATABASE_WITH_FILTER", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_QUERY_DATA_SOURCE", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_RETRIEVE_PAGE", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_RETRIEVE_COMMENT", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_RETRIEVE_DATABASE_PROPERTY", scope: ToolScope::Read },
    // ── Read: profile / users / files ───────────────────────────────
    CuratedTool { slug: "NOTION_LIST_USERS", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_GET_ABOUT_USER", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_GET_ABOUT_ME", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_LIST_FILE_UPLOADS", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_RETRIEVE_FILE_UPLOAD", scope: ToolScope::Read },
    CuratedTool { slug: "NOTION_LIST_DATA_SOURCE_TEMPLATES", scope: ToolScope::Read },
    // ── Write: create ───────────────────────────────────────────────
    CuratedTool { slug: "NOTION_CREATE_NOTION_PAGE", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_CREATE_DATABASE", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_CREATE_COMMENT", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_CREATE_FILE_UPLOAD", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_SEND_FILE_UPLOAD", scope: ToolScope::Write },
    // ── Write: update / append ──────────────────────────────────────
    CuratedTool { slug: "NOTION_UPDATE_PAGE", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_UPDATE_BLOCK", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_UPDATE_ROW_DATABASE", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_INSERT_ROW_DATABASE", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_INSERT_ROW_FROM_NL", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_REPLACE_PAGE_CONTENT", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_ADD_PAGE_CONTENT", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_ADD_MULTIPLE_PAGE_CONTENT", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_APPEND_BLOCK_CHILDREN", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_APPEND_TEXT_BLOCKS", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_APPEND_TASK_BLOCKS", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_APPEND_CODE_BLOCKS", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_APPEND_MEDIA_BLOCKS", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_APPEND_LAYOUT_BLOCKS", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_APPEND_TABLE_BLOCKS", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_DUPLICATE_PAGE", scope: ToolScope::Write },
    CuratedTool { slug: "NOTION_MOVE_PAGE", scope: ToolScope::Write },
    // ── Admin: destructive ──────────────────────────────────────────
    CuratedTool { slug: "NOTION_DELETE_BLOCK", scope: ToolScope::Admin },
    CuratedTool { slug: "NOTION_ARCHIVE_NOTION_PAGE", scope: ToolScope::Admin },
];
