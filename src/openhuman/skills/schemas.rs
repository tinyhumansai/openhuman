//! JSON-RPC / CLI controller surface for the skills domain.
//!
//! Exposes:
//! * `skills.list` — enumerate SKILL.md / legacy skills discovered in the
//!   current user home and workspace.
//! * `skills.read_resource` — read a single bundled resource file, with path
//!   traversal, symlink, size and UTF-8 guards.
//!
//! Both controllers resolve the active workspace via the persisted config
//! layer (`config::load_config_with_timeout`) so the CLI and UI see the same
//! skills catalog without the caller having to thread a workspace path.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::Config;
use crate::openhuman::skills::ops::{
    discover_skills, is_workspace_trusted, read_skill_resource, Skill, SkillScope,
};
use crate::rpc::RpcOutcome;

#[derive(Debug, Deserialize, Default)]
struct SkillsListParams {
    // No params today. Kept as an empty struct so future filters (scope,
    // search, etc.) can slot in without breaking older clients.
}

#[derive(Debug, Deserialize)]
struct SkillsReadResourceParams {
    skill_id: String,
    relative_path: String,
}

/// Wire-format representation of a discovered skill. Mirrors the fields in
/// [`Skill`] that are useful to the UI while hiding the
/// `frontmatter` blob (which includes a flatten'd forward-compat hatch and
/// can balloon with arbitrary YAML).
#[derive(Debug, Serialize)]
struct SkillSummary {
    id: String,
    name: String,
    description: String,
    version: String,
    author: Option<String>,
    tags: Vec<String>,
    tools: Vec<String>,
    prompts: Vec<String>,
    location: Option<String>,
    resources: Vec<String>,
    scope: SkillScope,
    legacy: bool,
    warnings: Vec<String>,
}

impl From<Skill> for SkillSummary {
    fn from(s: Skill) -> Self {
        SkillSummary {
            id: s.name.clone(),
            name: s.name,
            description: s.description,
            version: s.version,
            author: s.author,
            tags: s.tags,
            tools: s.tools,
            prompts: s.prompts,
            location: s.location.as_ref().map(|p| p.display().to_string()),
            resources: s
                .resources
                .into_iter()
                .map(|p| p.display().to_string())
                .collect(),
            scope: s.scope,
            legacy: s.legacy,
            warnings: s.warnings,
        }
    }
}

#[derive(Debug, Serialize)]
struct SkillsListResult {
    skills: Vec<SkillSummary>,
}

#[derive(Debug, Serialize)]
struct SkillsReadResourceResult {
    skill_id: String,
    relative_path: String,
    content: String,
    bytes: usize,
}

pub fn all_skills_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        skills_schemas("skills_list"),
        skills_schemas("skills_read_resource"),
    ]
}

pub fn all_skills_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: skills_schemas("skills_list"),
            handler: handle_skills_list,
        },
        RegisteredController {
            schema: skills_schemas("skills_read_resource"),
            handler: handle_skills_read_resource,
        },
    ]
}

pub fn skills_schemas(function: &str) -> ControllerSchema {
    match function {
        "skills_list" => ControllerSchema {
            namespace: "skills",
            function: "list",
            description: "List SKILL.md and legacy skills discovered in the user home and workspace.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "skills",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("SkillSummary"))),
                comment: "Discovered skills (sorted by name, project-scope shadows user-scope).",
                required: true,
            }],
        },
        "skills_read_resource" => ControllerSchema {
            namespace: "skills",
            function: "read_resource",
            description: "Read a single bundled SKILL resource file, hardened against traversal, symlink escape, and oversized payloads.",
            inputs: vec![
                FieldSchema {
                    name: "skill_id",
                    ty: TypeSchema::String,
                    comment: "Name of the skill (matches SkillSummary.id / Skill.name).",
                    required: true,
                },
                FieldSchema {
                    name: "relative_path",
                    ty: TypeSchema::String,
                    comment: "Path to the resource file, relative to the skill root (e.g. 'scripts/foo.sh').",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "skill_id",
                    ty: TypeSchema::String,
                    comment: "Echo of the requested skill id.",
                    required: true,
                },
                FieldSchema {
                    name: "relative_path",
                    ty: TypeSchema::String,
                    comment: "Echo of the requested relative path.",
                    required: true,
                },
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "File contents (UTF-8, <= 128 KB).",
                    required: true,
                },
                FieldSchema {
                    name: "bytes",
                    ty: TypeSchema::U64,
                    comment: "Size of the file on disk, in bytes.",
                    required: true,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: "skills",
            function: "unknown",
            description: "Unknown skills controller.",
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

fn handle_skills_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = deserialize_params::<SkillsListParams>(params)?;
        tracing::debug!("[skills][rpc] list skills");
        let workspace = resolve_workspace_dir().await;
        let trusted = is_workspace_trusted(&workspace);
        let home = dirs::home_dir();
        let skills = discover_skills(home.as_deref(), Some(workspace.as_path()), trusted);
        tracing::debug!(
            count = skills.len(),
            workspace = %workspace.display(),
            trusted,
            "[skills][rpc] list result"
        );
        let summaries = skills.into_iter().map(SkillSummary::from).collect();
        to_json(RpcOutcome::new(
            SkillsListResult { skills: summaries },
            Vec::new(),
        ))
    })
}

fn handle_skills_read_resource(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<SkillsReadResourceParams>(params)?;
        tracing::debug!(
            skill_id = %payload.skill_id,
            relative_path = %payload.relative_path,
            "[skills][rpc] read_resource"
        );
        let workspace = resolve_workspace_dir().await;
        let relative = Path::new(&payload.relative_path);
        match read_skill_resource(workspace.as_path(), &payload.skill_id, relative) {
            Ok(content) => {
                let bytes = content.len();
                to_json(RpcOutcome::new(
                    SkillsReadResourceResult {
                        skill_id: payload.skill_id,
                        relative_path: payload.relative_path,
                        content,
                        bytes,
                    },
                    Vec::new(),
                ))
            }
            Err(err) => {
                tracing::debug!(
                    error = %err,
                    "[skills][rpc] read_resource: rejected"
                );
                Err(err)
            }
        }
    })
}

/// Resolve the active workspace directory. Falls back to the runtime default
/// if the persisted config fails to load so the CLI and headless diagnostics
/// still work in partially-initialized environments.
async fn resolve_workspace_dir() -> PathBuf {
    match tokio::time::timeout(std::time::Duration::from_secs(30), Config::load_or_init()).await {
        Ok(Ok(cfg)) => cfg.workspace_dir,
        Ok(Err(err)) => {
            tracing::debug!(
                error = %err,
                "[skills][rpc] config load failed; falling back to default workspace"
            );
            fallback_workspace_dir()
        }
        Err(_) => {
            tracing::debug!(
                "[skills][rpc] config load timed out; falling back to default workspace"
            );
            fallback_workspace_dir()
        }
    }
}

fn fallback_workspace_dir() -> PathBuf {
    crate::openhuman::config::default_root_openhuman_dir()
        .unwrap_or_else(|_| PathBuf::from(".openhuman"))
        .join("workspace")
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_names_are_stable() {
        let list = skills_schemas("skills_list");
        assert_eq!(list.namespace, "skills");
        assert_eq!(list.function, "list");

        let read = skills_schemas("skills_read_resource");
        assert_eq!(read.namespace, "skills");
        assert_eq!(read.function, "read_resource");
    }

    #[test]
    fn controller_lists_match_lengths() {
        assert_eq!(
            all_skills_controller_schemas().len(),
            all_skills_registered_controllers().len()
        );
    }

    #[test]
    fn skill_summary_round_trip_minimum_fields() {
        let skill = Skill {
            name: "demo".to_string(),
            description: "desc".to_string(),
            version: "".to_string(),
            ..Default::default()
        };
        let summary: SkillSummary = skill.into();
        assert_eq!(summary.id, "demo");
        assert_eq!(summary.name, "demo");
        assert_eq!(summary.description, "desc");
    }
}
