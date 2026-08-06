//! Controller schema definitions for `openhuman.skill_registry_*` RPC methods.

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::handlers::{
    handle_browse, handle_categories, handle_install, handle_schemas, handle_search,
    handle_sources, handle_uninstall,
};

pub fn all_skill_registry_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        skill_registry_schemas("browse"),
        skill_registry_schemas("search"),
        skill_registry_schemas("sources"),
        skill_registry_schemas("categories"),
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
            description: "Browse the skill registry catalog (aggregated from HermesHub). Returns cached results unless force_refresh is true.",
            inputs: vec![FieldSchema {
                name: "force_refresh",
                ty: TypeSchema::Bool,
                comment: "Force re-fetch from the Hermes API, ignoring the local cache.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "entries",
                ty: TypeSchema::Json,
                comment: "Array of catalog entries.",
                required: true,
            }],
        },
        "search" => ControllerSchema {
            namespace: "skill_registry",
            function: "search",
            description: "Search the registry catalog by query string. Matches against name, description, tags, category, and author.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Search query string.",
                    required: false,
                },
                FieldSchema {
                    name: "source",
                    ty: TypeSchema::String,
                    comment: "Filter by upstream source (e.g. 'ClawHub', 'skills.sh', 'built-in').",
                    required: false,
                },
                FieldSchema {
                    name: "category",
                    ty: TypeSchema::String,
                    comment: "Filter by category.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "entries",
                ty: TypeSchema::Json,
                comment: "Matching catalog entries.",
                required: true,
            }],
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
        "install" => ControllerSchema {
            namespace: "skill_registry",
            function: "install",
            description: "Install a skill from the catalog by its entry id. Fetches the SKILL.md and installs to user scope.",
            inputs: vec![
                FieldSchema {
                    name: "entry_id",
                    ty: TypeSchema::String,
                    comment: "Catalog entry id of the skill to install.",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "url",
                    ty: TypeSchema::String,
                    comment: "The URL that was fetched.",
                    required: true,
                },
                FieldSchema {
                    name: "stdout",
                    ty: TypeSchema::String,
                    comment: "Diagnostic summary.",
                    required: true,
                },
                FieldSchema {
                    name: "stderr",
                    ty: TypeSchema::String,
                    comment: "Parse warnings.",
                    required: true,
                },
                FieldSchema {
                    name: "new_skills",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Slugs of skills that appeared post-install.",
                    required: true,
                },
            ],
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
