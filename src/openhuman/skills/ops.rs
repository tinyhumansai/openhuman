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
pub use super::ops_create::{create_skill, CreateSkillParams};
pub use super::ops_discover::{
    discover_skills, init_skills_dir, is_workspace_trusted, load_skills, read_skill_resource,
};
pub use super::ops_install::{
    install_skill_from_url, uninstall_skill, validate_install_url, validate_resolved_host,
    InstallSkillFromUrlOutcome, InstallSkillFromUrlParams, UninstallSkillOutcome,
    UninstallSkillParams, DEFAULT_INSTALL_TIMEOUT_SECS, MAX_INSTALL_TIMEOUT_SECS,
    MAX_INSTALL_URL_LEN, MAX_SKILL_MD_BYTES,
};
pub use super::ops_parse::{inventory_resources, parse_skill_md, parse_skill_md_str};
pub use super::ops_types::{Skill, SkillFrontmatter, SkillScope, MAX_SKILL_RESOURCE_BYTES};

#[cfg(test)]
pub(crate) use super::ops_create::{create_skill_inner, slugify_skill_name};
#[cfg(test)]
pub(crate) use super::ops_discover::discover_skills_inner;
#[cfg(test)]
pub(crate) use super::ops_install::{derive_install_slug, normalize_install_url};
#[cfg(test)]
pub(crate) use super::ops_types::{MAX_NAME_LEN, RESOURCE_DIRS, SKILL_MD, TRUST_MARKER};
#[cfg(test)]
pub(crate) use std::path::{Path, PathBuf};

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
