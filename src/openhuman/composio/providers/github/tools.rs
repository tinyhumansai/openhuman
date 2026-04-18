//! Curated catalog of GitHub Composio actions exposed to the agent.
//!
//! Composio publishes hundreds of GitHub actions; this hand-tuned slice
//! covers the day-to-day operations an AI assistant actually performs
//! (browsing repos, reading/writing issues + PRs, code search, basic
//! workflow control) and hides the long tail of admin endpoints.

use crate::openhuman::composio::providers::tool_scope::{CuratedTool, ToolScope};

pub const GITHUB_CURATED: &[CuratedTool] = &[
    // ── Read: user / repos ──────────────────────────────────────────
    CuratedTool {
        slug: "GITHUB_GET_AUTHENTICATED_USER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GITHUB_LIST_REPOSITORIES_FOR_AUTHENTICATED_USER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GITHUB_GET_A_REPOSITORY",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GITHUB_LIST_REPOSITORY_COLLABORATORS",
        scope: ToolScope::Read,
    },
    // ── Read: search ────────────────────────────────────────────────
    CuratedTool {
        slug: "GITHUB_SEARCH_REPOSITORIES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GITHUB_SEARCH_CODE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GITHUB_SEARCH_ISSUES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GITHUB_SEARCH_USERS",
        scope: ToolScope::Read,
    },
    // ── Read: issues ────────────────────────────────────────────────
    CuratedTool {
        slug: "GITHUB_LIST_REPOSITORY_ISSUES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GITHUB_GET_AN_ISSUE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GITHUB_LIST_ISSUE_COMMENTS",
        scope: ToolScope::Read,
    },
    // ── Read: pull requests ─────────────────────────────────────────
    CuratedTool {
        slug: "GITHUB_LIST_PULL_REQUESTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GITHUB_GET_A_PULL_REQUEST",
        scope: ToolScope::Read,
    },
    // CuratedTool { slug: "GITHUB_CHECK_IF_PULL_REQUEST_HAS_BEEN_MERGED", scope: ToolScope::Read },
    // ── Read: branches / commits ────────────────────────────────────
    CuratedTool {
        slug: "GITHUB_LIST_BRANCHES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GITHUB_GET_A_BRANCH",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GITHUB_LIST_COMMITS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GITHUB_GET_A_COMMIT",
        scope: ToolScope::Read,
    },
    // CuratedTool { slug: "GITHUB_COMPARE_TWO_COMMITS", scope: ToolScope::Read },
    // // ── Read: contents / releases / gists ───────────────────────────
    // CuratedTool { slug: "GITHUB_GET_REPOSITORY_CONTENTS", scope: ToolScope::Read },
    // CuratedTool { slug: "GITHUB_LIST_RELEASES", scope: ToolScope::Read },
    // CuratedTool { slug: "GITHUB_LIST_GISTS", scope: ToolScope::Read },
    // // ── Read: workflows ─────────────────────────────────────────────
    // CuratedTool { slug: "GITHUB_LIST_WORKFLOWS", scope: ToolScope::Read },
    // CuratedTool { slug: "GITHUB_LIST_WORKFLOW_RUNS", scope: ToolScope::Read },
    // ── Write: repos / contents ─────────────────────────────────────
    CuratedTool {
        slug: "GITHUB_CREATE_A_REPOSITORY_FOR_AUTHENTICATED_USER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_CREATE_OR_UPDATE_FILE_CONTENTS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_CREATE_A_COMMIT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_COMMIT_MULTIPLE_FILES",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_CREATE_A_COMMIT_COMMENT",
        scope: ToolScope::Write,
    },
    // ── Write: issues ───────────────────────────────────────────────
    CuratedTool {
        slug: "GITHUB_CREATE_AN_ISSUE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_UPDATE_AN_ISSUE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_CLOSE_AN_ISSUE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_CREATE_AN_ISSUE_COMMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_ADD_LABELS_TO_AN_ISSUE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_ADD_ASSIGNEES_TO_AN_ISSUE",
        scope: ToolScope::Write,
    },
    // ── Write: pull requests ────────────────────────────────────────
    CuratedTool {
        slug: "GITHUB_CREATE_A_PULL_REQUEST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_UPDATE_A_PULL_REQUEST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_MERGE_A_PULL_REQUEST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_CREATE_A_REVIEW_FOR_A_PULL_REQUEST",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GITHUB_CREATE_A_REVIEW_COMMENT_FOR_A_PULL_REQUEST",
        scope: ToolScope::Write,
    },
    // // ── Write: releases / gists / workflows ─────────────────────────
    // CuratedTool { slug: "GITHUB_CREATE_A_RELEASE", scope: ToolScope::Write },
    CuratedTool {
        slug: "GITHUB_CREATE_A_GIST",
        scope: ToolScope::Write,
    },
    // CuratedTool { slug: "GITHUB_CREATE_WORKFLOW_DISPATCH", scope: ToolScope::Write },
    // ── Admin: destructive / permission-changing ────────────────────
    CuratedTool {
        slug: "GITHUB_DELETE_A_REPOSITORY",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GITHUB_DELETE_A_BRANCH",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GITHUB_DELETE_A_FILE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GITHUB_ADD_A_REPOSITORY_COLLABORATOR",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GITHUB_CANCEL_WORKFLOW_RUN",
        scope: ToolScope::Admin,
    },
];
