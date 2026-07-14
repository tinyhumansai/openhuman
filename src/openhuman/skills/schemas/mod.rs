//! JSON-RPC / CLI controller surface for the skills domain.
//!
//! Exposes:
//! * `skills.list` — enumerate SKILL.md / legacy skills discovered in the
//!   current user home and workspace.
//! * `skills.read_resource` — read a single bundled resource file, with path
//!   traversal, symlink, size and UTF-8 guards.
//! * `skills.create` — scaffold a new SKILL.md skill under the user or
//!   workspace scope.
//! * `skills.install_from_url` — install a remote skill by fetching its
//!   `SKILL.md` over HTTPS (size-capped, timeout-clamped) and writing it into
//!   the user-scope skills directory. Rejects non-https, private-IP, and
//!   non-SKILL.md URLs; normalises `github.com/.../blob/...` → raw.
//!
//! All controllers resolve the active workspace via the persisted config
//! layer (`config::load_config_with_timeout`) so the CLI and UI see the same
//! skills catalog without the caller having to thread a workspace path.
//!
//! ## Sub-module layout
//!
//! | Module                | Lines  | Role                                                        |
//! |-----------------------|--------|-------------------------------------------------------------|
//! | `wire_types`          | ~200   | Param / result structs and `WorkflowSummary`.               |
//! | `helpers`             | ~80    | Config/workspace resolution + `deserialize_params`/`to_json`.|
//! | `handlers`            | ~240   | Thin `handle_*` dispatcher functions.                       |
//! | `controller_schemas`  | ~300   | `skills_schemas` match + `all_*` registry functions.     |

mod controller_schemas;
mod handlers;
mod helpers;
mod wire_types;

// ── External API — preserved exactly from the original schemas.rs ─────────────

pub use controller_schemas::{
    all_skills_controller_schemas, all_skills_registered_controllers, skills_schemas,
};

// `WorkflowSummary` is used by the unit tests.
#[cfg(test)]
pub(crate) use wire_types::WorkflowSummary;

// `Workflow` is used by the unit tests (skill_summary_round_trip_minimum_fields).
#[cfg(test)]
pub(crate) use crate::openhuman::skills::ops::Workflow;

// `resolve_workspace_dir` is used by the `run_workflow` agent tool.
pub(crate) use helpers::resolve_workspace_dir;

#[cfg(test)]
#[path = "../schemas_tests.rs"]
mod tests;
