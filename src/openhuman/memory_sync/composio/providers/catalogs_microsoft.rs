//! Curated catalogs — Microsoft personal-productivity toolkits:
//! OneDrive (files) and Excel (spreadsheets).
//!
//! These toolkits are catalog-only: they don't ship a native
//! [`super::ComposioProvider`] implementation, so they have no
//! user-profile fetch, no initial/periodic sync, no trigger webhooks,
//! and no memory ingestion. Connecting them via the UI lets the agent
//! invoke the listed actions through Composio's API, but their data
//! is not pre-ingested into OpenHuman's memory tree.
//!
//! Action slugs are sourced best-effort from
//! `https://docs.composio.dev/toolkits/<id>.md`. Slugs that don't
//! exist on the backend simply never appear in `composio_list_tools`,
//! so over-shooting is harmless.

use super::tool_scope::{CuratedTool, ToolScope};

// ── onedrive ────────────────────────────────────────────────────────
pub const ONE_DRIVE_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "ONE_DRIVE_GET_FILE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ONE_DRIVE_GET_FILE_METADATA",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ONE_DRIVE_LIST_FILES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ONE_DRIVE_LIST_CHILDREN",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ONE_DRIVE_SEARCH_FILES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ONE_DRIVE_DOWNLOAD_FILE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ONE_DRIVE_GET_DRIVE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "ONE_DRIVE_UPLOAD_FILE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ONE_DRIVE_CREATE_FOLDER",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ONE_DRIVE_COPY_FILE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ONE_DRIVE_MOVE_FILE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ONE_DRIVE_UPDATE_FILE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ONE_DRIVE_CREATE_SHARE_LINK",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "ONE_DRIVE_DELETE_FILE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "ONE_DRIVE_DELETE_FOLDER",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "ONE_DRIVE_RESTORE_FILE",
        scope: ToolScope::Admin,
    },
];

// ── excel ───────────────────────────────────────────────────────────
pub const EXCEL_CURATED: &[CuratedTool] = &[
    CuratedTool {
        slug: "EXCEL_GET_WORKBOOK",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "EXCEL_LIST_WORKSHEETS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "EXCEL_GET_WORKSHEET",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "EXCEL_GET_RANGE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "EXCEL_GET_USED_RANGE",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "EXCEL_LIST_TABLES",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "EXCEL_GET_TABLE_ROWS",
        scope: ToolScope::Read,
    },
    CuratedTool {
        slug: "EXCEL_CREATE_WORKSHEET",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "EXCEL_UPDATE_RANGE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "EXCEL_APPEND_ROWS",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "EXCEL_INSERT_TABLE_ROW",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "EXCEL_UPDATE_TABLE_ROW",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "EXCEL_CREATE_TABLE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "EXCEL_FORMAT_RANGE",
        scope: ToolScope::Write,
    },
    CuratedTool {
        slug: "EXCEL_DELETE_WORKSHEET",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "EXCEL_DELETE_TABLE",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "EXCEL_DELETE_TABLE_ROW",
        scope: ToolScope::Admin,
    },
    CuratedTool {
        slug: "EXCEL_CLEAR_RANGE",
        scope: ToolScope::Admin,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_drive_catalog_is_non_empty_and_unique() {
        assert!(!ONE_DRIVE_CURATED.is_empty());
        let mut slugs: Vec<&'static str> = ONE_DRIVE_CURATED.iter().map(|t| t.slug).collect();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), ONE_DRIVE_CURATED.len());
        for tool in ONE_DRIVE_CURATED {
            assert!(tool.slug.starts_with("ONE_DRIVE_"));
        }
    }

    #[test]
    fn excel_catalog_is_non_empty_and_unique() {
        assert!(!EXCEL_CURATED.is_empty());
        let mut slugs: Vec<&'static str> = EXCEL_CURATED.iter().map(|t| t.slug).collect();
        slugs.sort_unstable();
        slugs.dedup();
        assert_eq!(slugs.len(), EXCEL_CURATED.len());
        for tool in EXCEL_CURATED {
            assert!(tool.slug.starts_with("EXCEL_"));
        }
    }

    #[test]
    fn one_drive_catalog_covers_all_three_scopes() {
        assert!(ONE_DRIVE_CURATED.iter().any(|t| t.scope == ToolScope::Read));
        assert!(ONE_DRIVE_CURATED
            .iter()
            .any(|t| t.scope == ToolScope::Write));
        assert!(ONE_DRIVE_CURATED
            .iter()
            .any(|t| t.scope == ToolScope::Admin));
    }

    #[test]
    fn excel_catalog_covers_all_three_scopes() {
        assert!(EXCEL_CURATED.iter().any(|t| t.scope == ToolScope::Read));
        assert!(EXCEL_CURATED.iter().any(|t| t.scope == ToolScope::Write));
        assert!(EXCEL_CURATED.iter().any(|t| t.scope == ToolScope::Admin));
    }
}
