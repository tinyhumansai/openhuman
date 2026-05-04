//! Curated catalogs — Google toolkits: GoogleCalendar, GoogleDrive,
//! GoogleDocs, GoogleSheets.

use super::tool_scope::{CuratedTool, ToolScope};

// ── googlecalendar ──────────────────────────────────────────────────
pub const GOOGLECALENDAR_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "GOOGLECALENDAR_EVENTS_LIST",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_FIND_EVENT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_LIST_CALENDARS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_EVENTS_GET",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_FIND_FREE_SLOTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_GET_CALENDAR",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_EVENTS_LIST_ALL_CALENDARS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_CREATE_EVENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_UPDATE_EVENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_PATCH_EVENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_QUICK_ADD",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_EVENTS_MOVE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_REMOVE_ATTENDEE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_EVENTS_IMPORT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_DELETE_EVENT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_CLEAR_CALENDAR",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_CALENDARS_DELETE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_DUPLICATE_CALENDAR",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_PATCH_CALENDAR",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_ACL_INSERT",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLECALENDAR_ACL_DELETE",
        scope: ToolScope::Admin,
    },
];

// ── googledrive ─────────────────────────────────────────────────────
pub const GOOGLEDRIVE_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "GOOGLEDRIVE_FIND_FILE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_LIST_FILES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_GET_FILE_METADATA",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_DOWNLOAD_FILE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_LIST_PERMISSIONS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_FIND_FOLDER",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_GET_ABOUT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_CREATE_FILE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_CREATE_FOLDER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_UPLOAD_FILE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_CREATE_FILE_FROM_TEXT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_COPY_FILE_ADVANCED",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_MOVE_FILE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_EDIT_FILE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_RENAME_FILE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_CREATE_PERMISSION",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_DELETE_PERMISSION",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_UPDATE_PERMISSION",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_DELETE_FILE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_GOOGLE_DRIVE_DELETE_FOLDER_OR_FILE_ACTION",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLEDRIVE_EMPTY_TRASH",
        scope: ToolScope::Admin,
    },
];

// ── googledocs ──────────────────────────────────────────────────────
pub const GOOGLEDOCS_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "GOOGLEDOCS_GET_DOCUMENT_BY_ID",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_GET_DOCUMENT_PLAINTEXT",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_SEARCH_DOCUMENTS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_CREATE_DOCUMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_CREATE_DOCUMENT_MARKDOWN",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_INSERT_TEXT_ACTION",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_INSERT_TABLE_ACTION",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_INSERT_INLINE_IMAGE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_UPDATE_EXISTING_DOCUMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_UPDATE_DOCUMENT_MARKDOWN",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_UPDATE_DOCUMENT_SECTION_MARKDOWN",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_REPLACE_ALL_TEXT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_COPY_DOCUMENT",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_CREATE_HEADER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_CREATE_FOOTER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_DELETE_CONTENT_RANGE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_DELETE_HEADER",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_DELETE_FOOTER",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_DELETE_NAMED_RANGE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_DELETE_TABLE_ROW",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLEDOCS_DELETE_TABLE_COLUMN",
        scope: ToolScope::Admin,
    },
];

// ── googlesheets ────────────────────────────────────────────────────
pub const GOOGLESHEETS_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "GOOGLESHEETS_BATCH_GET",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_VALUES_GET",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_LOOKUP_SPREADSHEET_ROW",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_GET_SPREADSHEET_INFO",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_GET_SHEET_NAMES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_SEARCH_SPREADSHEETS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_VALUES_UPDATE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_UPDATE_VALUES_BATCH",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_SPREADSHEETS_VALUES_APPEND",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_UPSERT_ROWS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_CREATE_GOOGLE_SHEET1",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_ADD_SHEET",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_CREATE_SPREADSHEET_ROW",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_CREATE_SPREADSHEET_COLUMN",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_FIND_REPLACE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_FORMAT_CELL",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_SET_DATA_VALIDATION_RULE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_SPREADSHEETS_VALUES_BATCH_CLEAR",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_DELETE_SHEET",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_DELETE_DIMENSION",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_UPDATE_SHEET_PROPERTIES",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "GOOGLESHEETS_UPDATE_SPREADSHEET_PROPERTIES",
        scope: ToolScope::Admin,
    },
];
