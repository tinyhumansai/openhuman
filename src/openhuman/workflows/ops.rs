//! Discovery and parsing of agentskills.io-style skills.
//!
//! A skill is a directory containing a `SKILL.md` file with YAML frontmatter
//! (`name`, `description`, …) followed by Markdown instructions. Optional
//! bundled resources live in sibling subdirectories (`scripts/`, `references/`,
//! `assets/`).
//!
//! Skills can be installed at two scopes:
//! - **User**: `~/.openhuman/skills/<name>/` or `~/.agents/skills/<name>/`
//! - **Project**: `<workspace>/.openhuman/skills/<name>/` or
//!   `<workspace>/.agents/skills/<name>/`
//!
//! Project-scope skills are only loaded when a trust marker
//! (`<workspace>/.openhuman/trust`) is present. When a skill name collides
//! across scopes, the project-scope copy wins.
//!
//! Legacy `skill.json` manifests and the flat `<workspace>/skills/<name>/`
//! layout are still supported for backward compatibility.
//!
//! ## Module layout
//!
//! | Module | Contents |
//! |---|---|
//! | [`super::ops_types`] | Core types, constants, and frontmatter helpers |
//! | [`super::ops_discover`] | Scanning root directories, scope resolution, collision handling |
//! | [`super::ops_parse`] | SKILL.md parsing, resource inventory, skill-resource reading |
//! | [`super::ops_create`] | Scaffolding new SKILL.md-based skills on disk |
//! | [`super::ops_install`] | URL-based skill installation over HTTPS |

// Re-export everything that was previously public from this file so external
// callers are unaffected.
pub use super::ops_create::{create_workflow, CreateWorkflowParams, WorkflowCreateInputDef};
pub use super::ops_discover::{
    discover_automations, discover_workflows, init_workflows_dir, is_workspace_trusted,
    load_workflow_metadata, read_workflow_resource,
};
pub use super::ops_install::{
    install_workflow_from_url, uninstall_workflow, validate_install_url, validate_resolved_host,
    InstallWorkflowFromUrlOutcome, InstallWorkflowFromUrlParams, UninstallWorkflowOutcome,
    UninstallWorkflowParams, DEFAULT_INSTALL_TIMEOUT_SECS, MAX_INSTALL_TIMEOUT_SECS,
    MAX_INSTALL_URL_LEN, MAX_WORKFLOW_MD_BYTES,
};
pub use super::ops_parse::{inventory_resources, parse_workflow_md, parse_workflow_md_str};
pub use super::ops_types::{
    Workflow, WorkflowFrontmatter, WorkflowScope, MAX_WORKFLOW_RESOURCE_BYTES,
};

#[cfg(test)]
pub(crate) use super::ops_create::{create_workflow_inner, slugify_workflow_name};
#[cfg(test)]
pub(crate) use super::ops_discover::discover_workflows_inner;
#[cfg(test)]
pub(crate) use super::ops_install::{
    derive_install_slug, install_workflow_from_url_with_home, normalize_install_url,
};
#[cfg(test)]
pub(crate) use super::ops_types::{
    MAX_NAME_LEN, RESOURCE_DIRS, SKILL_MD, TRUST_MARKER, WORKFLOW_MD, WORKFLOW_TOML,
};
#[cfg(test)]
pub(crate) use std::path::{Path, PathBuf};

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
