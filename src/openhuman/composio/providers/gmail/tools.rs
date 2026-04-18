//! Curated catalog of Gmail Composio actions exposed to the agent.
//!
//! Composio publishes 60+ Gmail actions; this hand-tuned slice covers
//! the cases the agent actually plans for (read, compose, manage) and
//! hides the long tail of edge-case admin endpoints.

use crate::openhuman::composio::providers::tool_scope::{CuratedTool, ToolScope};

pub const GMAIL_CURATED: &[CuratedTool] = &[
    // ── Read: messages & threads ────────────────────────────────────
    CuratedTool { slug: "GMAIL_FETCH_EMAILS", scope: ToolScope::Read },
    CuratedTool { slug: "GMAIL_LIST_MESSAGES", scope: ToolScope::Read },
    CuratedTool { slug: "GMAIL_FETCH_MESSAGE_BY_MESSAGE_ID", scope: ToolScope::Read },
    CuratedTool { slug: "GMAIL_FETCH_MESSAGE_BY_THREAD_ID", scope: ToolScope::Read },
    CuratedTool { slug: "GMAIL_LIST_THREADS", scope: ToolScope::Read },
    CuratedTool { slug: "GMAIL_GET_ATTACHMENT", scope: ToolScope::Read },
    // ── Read: profile & settings ────────────────────────────────────
    CuratedTool { slug: "GMAIL_GET_PROFILE", scope: ToolScope::Read },
    // CuratedTool { slug: "GMAIL_GET_LANGUAGE_SETTINGS", scope: ToolScope::Read },
    // CuratedTool { slug: "GMAIL_GET_VACATION_SETTINGS", scope: ToolScope::Read },
    // CuratedTool { slug: "GMAIL_GET_AUTO_FORWARDING", scope: ToolScope::Read },
    // ── Read: contacts & people ─────────────────────────────────────
    CuratedTool { slug: "GMAIL_GET_CONTACTS", scope: ToolScope::Read },
    CuratedTool { slug: "GMAIL_GET_PEOPLE", scope: ToolScope::Read },
    CuratedTool { slug: "GMAIL_SEARCH_PEOPLE", scope: ToolScope::Read },
    // ── Read: drafts & labels ───────────────────────────────────────
    CuratedTool { slug: "GMAIL_LIST_DRAFTS", scope: ToolScope::Read },
    CuratedTool { slug: "GMAIL_GET_DRAFT", scope: ToolScope::Read },
    CuratedTool { slug: "GMAIL_LIST_LABELS", scope: ToolScope::Read },
    CuratedTool { slug: "GMAIL_GET_LABEL", scope: ToolScope::Read },
    // ── Write: send & compose ───────────────────────────────────────
    CuratedTool { slug: "GMAIL_SEND_EMAIL", scope: ToolScope::Write },
    CuratedTool { slug: "GMAIL_REPLY_TO_THREAD", scope: ToolScope::Write },
    CuratedTool { slug: "GMAIL_FORWARD_MESSAGE", scope: ToolScope::Write },
    // ── Write: drafts ───────────────────────────────────────────────
    CuratedTool { slug: "GMAIL_CREATE_EMAIL_DRAFT", scope: ToolScope::Write },
    CuratedTool { slug: "GMAIL_UPDATE_DRAFT", scope: ToolScope::Write },
    CuratedTool { slug: "GMAIL_SEND_DRAFT", scope: ToolScope::Write },
    // ── Write: labels (create/update on user labels) ────────────────
    // CuratedTool { slug: "GMAIL_CREATE_LABEL", scope: ToolScope::Write },
    // CuratedTool { slug: "GMAIL_UPDATE_LABEL", scope: ToolScope::Write },
    // CuratedTool { slug: "GMAIL_PATCH_LABEL", scope: ToolScope::Write },
    CuratedTool { slug: "GMAIL_ADD_LABEL_TO_EMAIL", scope: ToolScope::Write },
    // ── Admin: destructive & permission-changing ────────────────────
    CuratedTool { slug: "GMAIL_DELETE_MESSAGE", scope: ToolScope::Admin },
    CuratedTool { slug: "GMAIL_BATCH_DELETE_MESSAGES", scope: ToolScope::Admin },
    CuratedTool { slug: "GMAIL_MOVE_TO_TRASH", scope: ToolScope::Admin },
    // CuratedTool { slug: "GMAIL_UNTRASH_MESSAGE", scope: ToolScope::Admin },
    CuratedTool { slug: "GMAIL_DELETE_THREAD", scope: ToolScope::Admin },
    CuratedTool { slug: "GMAIL_MOVE_THREAD_TO_TRASH", scope: ToolScope::Admin },
    CuratedTool { slug: "GMAIL_UNTRASH_THREAD", scope: ToolScope::Admin },
    // CuratedTool { slug: "GMAIL_MODIFY_THREAD_LABELS", scope: ToolScope::Admin },
    // CuratedTool { slug: "GMAIL_BATCH_MODIFY_MESSAGES", scope: ToolScope::Admin },
    CuratedTool { slug: "GMAIL_DELETE_DRAFT", scope: ToolScope::Admin },
    CuratedTool { slug: "GMAIL_DELETE_LABEL", scope: ToolScope::Admin },
    // CuratedTool { slug: "GMAIL_PATCH_SEND_AS", scope: ToolScope::Admin },
    // CuratedTool { slug: "GMAIL_UPDATE_IMAP_SETTINGS", scope: ToolScope::Admin },
];
