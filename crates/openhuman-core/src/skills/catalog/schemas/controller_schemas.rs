//! Controller schema definitions for `openhuman.skill_registry_*` RPC methods.

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::handlers::{
    handle_browse, handle_categories, handle_detail, handle_install, handle_schemas, handle_search,
    handle_sources, handle_uninstall,
};

pub fn all_skill_registry_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        skill_registry_schemas("browse"),
        skill_registry_schemas("search"),
        skill_registry_schemas("sources"),
        skill_registry_schemas("categories"),
        skill_registry_schemas("detail"),
        skill_registry_schemas("install"),
        skill_registry_schemas("uninstall"),
        skill_registry_schemas("schemas"),
    ]
}

pub fn all_skill_registry_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: skill_registry_schemas("browse"),
            handler: handle_browse,
        },
        RegisteredController {
            schema: skill_registry_schemas("search"),
            handler: handle_search,
        },
        RegisteredController {
            schema: skill_registry_schemas("sources"),
            handler: handle_sources,
        },
        RegisteredController {
            schema: skill_registry_schemas("categories"),
            handler: handle_categories,
        },
        RegisteredController {
            schema: skill_registry_schemas("detail"),
            handler: handle_detail,
        },
        RegisteredController {
            schema: skill_registry_schemas("install"),
            handler: handle_install,
        },
        RegisteredController {
            schema: skill_registry_schemas("uninstall"),
            handler: handle_uninstall,
        },
        RegisteredController {
            schema: skill_registry_schemas("schemas"),
            handler: handle_schemas,
        },
    ]
}

pub fn skill_registry_schemas(function: &str) -> ControllerSchema {
    match function {
        "browse" => ControllerSchema {
            namespace: "skill_registry",
            function: "browse",
            description: "Browse the skill registry catalog (aggregated from HermesHub). Serves the cached catalog and refreshes it in the background when stale; force_refresh refetches first. Pass page or page_size for one page; with neither, every entry is returned.",
            inputs: catalog_inputs(false),
            outputs: catalog_outputs("Catalog entries on this page."),
        },
        "search" => ControllerSchema {
            namespace: "skill_registry",
            function: "search",
            description: "Search the registry catalog by query string. Matches against name, description, tags, category, and author. Pass page or page_size for one page; with neither, every match is returned.",
            inputs: catalog_inputs(true),
            outputs: catalog_outputs("Matching catalog entries on this page."),
        },
        "sources" => ControllerSchema {
            namespace: "skill_registry",
            function: "sources",
            description: "List the distinct upstream sources present in the catalog (e.g. 'built-in', 'ClawHub', 'skills.sh').",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "sources",
                ty: TypeSchema::Json,
                comment: "Array of source name strings.",
                required: true,
            }],
        },
        "categories" => ControllerSchema {
            namespace: "skill_registry",
            function: "categories",
            description: "List the distinct categories present in the catalog.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "categories",
                ty: TypeSchema::Json,
                comment: "Array of category name strings.",
                required: true,
            }],
        },
        "detail" => ControllerSchema {
            namespace: "skill_registry",
            function: "detail",
            description: "Everything the registry knows about one catalog entry, by id (or a name exactly one entry carries).",
            inputs: vec![FieldSchema {
                name: "entry_id",
                ty: TypeSchema::String,
                comment: "Catalog entry id.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "entry",
                ty: TypeSchema::Json,
                comment: "The catalog entry fields plus registry, installable, category_label, overview and install_identifier.",
                required: true,
            }],
        },
        "install" => ControllerSchema {
            namespace: "skill_registry",
            function: "install",
            description: "Install a skill from the catalog by its entry id. Fetches the SKILL.md, runs the supply-chain scan (retrying once when it blocks or the fetch fails) and installs to user scope. A document whose scan still blocks is not installed: the result has status `scan_blocked` and the findings.",
            inputs: vec![
                FieldSchema {
                    name: "entry_id",
                    ty: TypeSchema::String,
                    comment: "Catalog entry id of the skill to install.",
                    required: true,
                },
                FieldSchema {
                    name: "acknowledged_digest",
                    ty: TypeSchema::String,
                    comment: "The `digest` of a `scan_blocked` result, sent by the Skills UI only after the user reviewed its findings and chose to install anyway. It installs that document only; a different document is scanned and refused afresh. Agent tools cannot set it.",
                    required: false,
                },
            ],
            outputs: install_outputs("new_skills"),
        },
        "uninstall" => ControllerSchema {
            namespace: "skill_registry",
            function: "uninstall",
            description: "Uninstall an installed user-scope skill by slug.",
            inputs: vec![FieldSchema {
                name: "name",
                ty: TypeSchema::String,
                comment: "Installed skill slug to remove from the user skills directory.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::String,
                    comment: "Removed skill slug.",
                    required: true,
                },
                FieldSchema {
                    name: "removed_path",
                    ty: TypeSchema::String,
                    comment: "Absolute path removed from disk.",
                    required: true,
                },
                FieldSchema {
                    name: "scope",
                    ty: TypeSchema::String,
                    comment: "Scope removed; currently user.",
                    required: true,
                },
            ],
        },
        "schemas" => ControllerSchema {
            namespace: "skill_registry",
            function: "schemas",
            description: "Return the skill_registry controller schemas for CLI/RPC smoke-test script generation.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "schemas",
                ty: TypeSchema::Json,
                comment: "Array of skill_registry controller schemas.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "skill_registry",
            function: "unknown",
            description: "Unknown skill_registry controller.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

/// The outputs of an install that may stop at the supply-chain scan.
pub(crate) fn install_outputs(new_field: &'static str) -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "status",
            ty: TypeSchema::String,
            comment: "`installed`, or `scan_blocked` when the scan still blocks after a retry and nothing was installed.",
            required: true,
        },
        FieldSchema {
            name: "url",
            ty: TypeSchema::String,
            comment: "The URL that was fetched (installed).",
            required: false,
        },
        FieldSchema {
            name: "stdout",
            ty: TypeSchema::String,
            comment: "Diagnostic summary (installed).",
            required: false,
        },
        FieldSchema {
            name: "stderr",
            ty: TypeSchema::String,
            comment: "Parse warnings (installed).",
            required: false,
        },
        FieldSchema {
            name: new_field,
            ty: TypeSchema::Array(Box::new(TypeSchema::String)),
            comment: "Slugs of skills that appeared post-install (installed).",
            required: false,
        },
        FieldSchema {
            name: "target",
            ty: TypeSchema::String,
            comment: "The entry id or URL that was refused (scan_blocked).",
            required: false,
        },
        FieldSchema {
            name: "fetched_from",
            ty: TypeSchema::String,
            comment: "The redacted URL the blocked document came from (scan_blocked).",
            required: false,
        },
        FieldSchema {
            name: "slug",
            ty: TypeSchema::String,
            comment: "The install slug the blocked document would have used (scan_blocked).",
            required: false,
        },
        FieldSchema {
            name: "digest",
            ty: TypeSchema::String,
            comment: "Digest of the blocked document, to send back as `acknowledged_digest` (scan_blocked).",
            required: false,
        },
        FieldSchema {
            name: "findings",
            ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
            comment: "Scan findings: `check`, `verdict`, `field` and `message` (scan_blocked).",
            required: false,
        },
        FieldSchema {
            name: "message",
            ty: TypeSchema::String,
            comment: "Why the install was refused (scan_blocked).",
            required: false,
        },
    ]
}

fn catalog_inputs(with_query: bool) -> Vec<FieldSchema> {
    let mut inputs = Vec::new();
    if with_query {
        inputs.push(FieldSchema {
            name: "query",
            ty: TypeSchema::String,
            comment: "Search query string.",
            required: false,
        });
    }
    inputs.extend([
        FieldSchema {
            name: "source",
            ty: TypeSchema::String,
            comment: "Filter by one upstream source (e.g. 'ClawHub', 'skills.sh', 'built-in').",
            required: false,
        },
        FieldSchema {
            name: "sources",
            ty: TypeSchema::Array(Box::new(TypeSchema::String)),
            comment: "Filter by any of these upstream sources.",
            required: false,
        },
        FieldSchema {
            name: "category",
            ty: TypeSchema::String,
            comment: "Filter by category.",
            required: false,
        },
        FieldSchema {
            name: "categories",
            ty: TypeSchema::Array(Box::new(TypeSchema::String)),
            comment: "Filter by any of these categories.",
            required: false,
        },
        FieldSchema {
            name: "page",
            ty: TypeSchema::U64,
            comment: "1-based page number.",
            required: false,
        },
        FieldSchema {
            name: "page_size",
            ty: TypeSchema::U64,
            comment: "Entries per page, 1-100 (default 25).",
            required: false,
        },
        FieldSchema {
            name: "force_refresh",
            ty: TypeSchema::Bool,
            comment: "Refetch the catalog before answering.",
            required: false,
        },
    ]);
    inputs
}

fn catalog_outputs(entries_comment: &'static str) -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "entries",
            ty: TypeSchema::Json,
            comment: entries_comment,
            required: true,
        },
        FieldSchema {
            name: "total",
            ty: TypeSchema::U64,
            comment: "Matches across all pages.",
            required: true,
        },
        FieldSchema {
            name: "page",
            ty: TypeSchema::U64,
            comment: "The 1-based page served.",
            required: true,
        },
        FieldSchema {
            name: "page_size",
            ty: TypeSchema::U64,
            comment: "Entries per page after clamping.",
            required: true,
        },
        FieldSchema {
            name: "total_pages",
            ty: TypeSchema::U64,
            comment: "Number of pages; 0 when nothing matched.",
            required: true,
        },
        FieldSchema {
            name: "freshness",
            ty: TypeSchema::Enum {
                variants: vec!["live", "cached", "local_fallback"],
            },
            comment: "How fresh the catalog behind this page is.",
            required: true,
        },
        FieldSchema {
            name: "fetched_at",
            ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
            comment: "Unix seconds of the oldest catalog fetch that answered.",
            required: false,
        },
        FieldSchema {
            name: "refreshing",
            ty: TypeSchema::Bool,
            comment: "Whether a background refresh is running.",
            required: true,
        },
        FieldSchema {
            name: "last_error",
            ty: TypeSchema::Json,
            comment: "The last refresh failure ({kind, message, retry_after_secs}), or null.",
            required: false,
        },
    ]
}
