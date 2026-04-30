//! Curated catalogs — productivity toolkits: Outlook, Linear, Jira,
//! Trello, Asana, Dropbox.

use super::tool_scope::{CuratedTool, ToolScope};

// ── outlook ─────────────────────────────────────────────────────────
pub const OUTLOOK_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "OUTLOOK_GET_MESSAGE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "OUTLOOK_LIST_MESSAGES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "OUTLOOK_SEARCH_MESSAGES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "OUTLOOK_LIST_CALENDARS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "OUTLOOK_LIST_CALENDAR_EVENTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "OUTLOOK_GET_CALENDAR_EVENT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "OUTLOOK_LIST_CONTACTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "OUTLOOK_LIST_MAIL_FOLDERS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "OUTLOOK_SEND_EMAIL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "OUTLOOK_CREATE_DRAFT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "OUTLOOK_SEND_DRAFT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "OUTLOOK_CREATE_DRAFT_REPLY",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "OUTLOOK_CREATE_ME_FORWARD_DRAFT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "OUTLOOK_CALENDAR_CREATE_EVENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "OUTLOOK_CREATE_CONTACT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "OUTLOOK_CREATE_MAIL_FOLDER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "OUTLOOK_DELETE_MESSAGE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "OUTLOOK_BATCH_MOVE_MESSAGES",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "OUTLOOK_BATCH_UPDATE_MESSAGES",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "OUTLOOK_ACCEPT_EVENT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "OUTLOOK_CANCEL_EVENT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "OUTLOOK_CREATE_ME_CALENDAR_PERMISSION",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "OUTLOOK_CREATE_EMAIL_RULE",
        scope: ToolScope::Admin,
    },
];

// ── linear ──────────────────────────────────────────────────────────
pub const LINEAR_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "LINEAR_LIST_LINEAR_ISSUES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "LINEAR_GET_LINEAR_ISSUE",
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
        slug: "LINEAR_LIST_LINEAR_STATES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "LINEAR_SEARCH_ISSUES",
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
    CuratedTool {
        slug: "LINEAR_GET_LINEAR_PROJECT",
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
        slug: "LINEAR_CREATE_ATTACHMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_CREATE_LINEAR_PROJECT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_CREATE_LINEAR_LABEL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_UPDATE_LINEAR_COMMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_CREATE_ISSUE_RELATION",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "LINEAR_UPDATE_LINEAR_PROJECT",
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

// ── jira ────────────────────────────────────────────────────────────
pub const JIRA_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "JIRA_GET_ISSUE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "JIRA_GET_ALL_PROJECTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "JIRA_FETCH_BULK_ISSUES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "JIRA_GET_ISSUE_TYPES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "JIRA_GET_PROJECT_ROLES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "JIRA_FIND_USERS2",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "JIRA_GET_FIELDS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "JIRA_GET_ISSUE_EDIT_METADATA",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "JIRA_GET_PROJECT_VERSIONS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "JIRA_CREATE_ISSUE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "JIRA_BULK_CREATE_ISSUE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "JIRA_EDIT_ISSUE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "JIRA_ADD_COMMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "JIRA_ASSIGN_ISSUE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "JIRA_ADD_ATTACHMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "JIRA_CREATE_ISSUE_LINK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "JIRA_ADD_WORKLOG",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "JIRA_TRANSITION_ISSUE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "JIRA_DELETE_ISSUE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "JIRA_DELETE_COMMENT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "JIRA_DELETE_VERSION",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "JIRA_DELETE_WORKLOG",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "JIRA_CREATE_PROJECT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "JIRA_ADD_USERS_TO_PROJECT_ROLE",
        scope: ToolScope::Admin,
    },
];

// ── trello ──────────────────────────────────────────────────────────
pub const TRELLO_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "TRELLO_GET_BOARDS_BY_ID_BOARD",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TRELLO_GET_ACTIONS_BY_ID_ACTION",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TRELLO_GET_BATCH",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TRELLO_GET_BOARDS_ACTIONS_BY_ID_BOARD",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TRELLO_GET_MEMBERS_BOARDS_BY_ID_MEMBER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "TRELLO_ADD_CARDS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TRELLO_ADD_BOARDS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TRELLO_ADD_LISTS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TRELLO_ADD_CARDS_ACTIONS_COMMENTS_BY_ID_CARD",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TRELLO_ADD_MEMBER_TO_CARD",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TRELLO_CREATE_CARD_LABEL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TRELLO_ADD_CARDS_ATTACHMENTS_BY_ID_CARD",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TRELLO_ADD_CARDS_CHECKLISTS_BY_ID_CARD",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TRELLO_CREATE_WEBHOOK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "TRELLO_DELETE_CARDS_BY_ID_CARD",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TRELLO_DELETE_BOARD",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TRELLO_DELETE_CHECKLISTS_BY_ID_CHECKLIST",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TRELLO_ARCHIVE_ALL_LIST_CARDS",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TRELLO_DELETE_CARD_COMMENT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TRELLO_DELETE_LABELS_BY_ID_LABEL",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TRELLO_DELETE_ORGANIZATIONS_BY_ID_ORG",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "TRELLO_DELETE_WEBHOOKS_BY_ID_WEBHOOK",
        scope: ToolScope::Admin,
    },
];

// ── asana ───────────────────────────────────────────────────────────
pub const ASANA_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "ASANA_GET_A_TASK",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ASANA_GET_A_PROJECT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ASANA_GET_MULTIPLE_TASKS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ASANA_GET_MULTIPLE_PROJECTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ASANA_GET_CURRENT_USER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ASANA_GET_MULTIPLE_WORKSPACES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ASANA_GET_PORTFOLIO",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ASANA_GET_GOALS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ASANA_GET_CUSTOM_FIELDS_FOR_WORKSPACE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ASANA_CREATE_A_TASK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ASANA_CREATE_A_PROJECT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ASANA_CREATE_SUBTASK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ASANA_CREATE_TASK_COMMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ASANA_UPDATE_A_TASK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ASANA_ADD_FOLLOWERS_TO_TASK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ASANA_ADD_TAG_TO_TASK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ASANA_ADD_PROJECT_FOR_TASK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ASANA_ADD_TASK_DEPENDENCIES",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ASANA_CREATE_ATTACHMENT_FOR_TASK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ASANA_DELETE_TASK",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "ASANA_DELETE_PROJECT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "ASANA_DELETE_SECTION",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "ASANA_DELETE_TAG",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "ASANA_DELETE_CUSTOM_FIELD",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "ASANA_DELETE_MEMBERSHIP",
        scope: ToolScope::Admin,
    },
];

// ── dropbox ─────────────────────────────────────────────────────────
pub const DROPBOX_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "DROPBOX_GET_METADATA",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "DROPBOX_FILES_SEARCH",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "DROPBOX_LIST_FILE_MEMBERS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "DROPBOX_GET_SHARED_LINK_METADATA",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "DROPBOX_GET_ABOUT_ME",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "DROPBOX_GET_SPACE_USAGE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "DROPBOX_ALPHA_UPLOAD_FILE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "DROPBOX_CREATE_FOLDER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "DROPBOX_COPY_FILE_OR_FOLDER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "DROPBOX_CREATE_SHARED_LINK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "DROPBOX_ADD_FILE_MEMBER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "DROPBOX_DELETE_FILE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "DROPBOX_DELETE_BATCH",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "DROPBOX_ADD_TEAM_MEMBERS",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "DROPBOX_CREATE_TEAM_FOLDER",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "DROPBOX_ARCHIVE_TEAM_FOLDER",
        scope: ToolScope::Admin,
    },
];
