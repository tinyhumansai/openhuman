//! Unified Skill Registry
//!
//! A single registry that aggregates both skill types:
//!
//! - `openhuman`: JavaScript-based skills executed in the QuickJS runtime.
//! - `openclaw`:   File-based skills defined in SKILL.md or SKILL.toml.
//!
//! All skills expose a common [`UnifiedSkillEntry`] interface and return
//! [`UnifiedSkillResult`] on execution, regardless of their underlying type.

pub mod generator;
pub mod llm_generator;
pub mod openclaw_executor;
pub mod self_evolve;
pub mod skill_tester;

use crate::openhuman::skills::{load_skills, Skill};
use crate::runtime::qjs_engine::RuntimeEngine;
use crate::runtime::types::{ToolDefinition, UnifiedSkillEntry, UnifiedSkillResult};
use chrono::Utc;
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// Specification for programmatically generating a new skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateSkillSpec {
    /// Display name for the skill (e.g., "My Skill").
    pub name: String,
    /// Human-readable description of what the skill does.
    pub description: String,
    /// Skill type: "openhuman" or "openclaw".
    pub skill_type: String,
    /// For openhuman skills: the JavaScript body of the generated tool function.
    pub tool_code: Option<String>,
    /// For openclaw skills: markdown content written to SKILL.md.
    pub markdown_content: Option<String>,
    /// For openclaw skills: shell command written into SKILL.toml as a tool.
    pub shell_command: Option<String>,
    /// Complete LLM-generated `index.js` source.  When present,
    /// `generator::generate_openhuman` writes this directly to disk instead
    /// of building from the default template.
    #[serde(default)]
    pub full_index_js: Option<String>,
}

/// The unified skill registry wrapping the QuickJS engine and openclaw loader.
pub struct UnifiedSkillRegistry {
    engine: Arc<RuntimeEngine>,
}

impl UnifiedSkillRegistry {
    pub fn new(engine: Arc<RuntimeEngine>) -> Self {
        Self { engine }
    }

    /// Return the resolved skills source directory (where openhuman skill
    /// directories are stored).
    pub fn skills_dir(&self) -> Result<PathBuf, String> {
        self.engine.skills_source_dir()
    }

    /// Return a clone of the inner `RuntimeEngine` Arc.
    pub fn engine(&self) -> Arc<RuntimeEngine> {
        Arc::clone(&self.engine)
    }

    /// List all skills from both subsystems.
    ///
    /// - openhuman skills come from `RuntimeEngine::discover_skills()` (manifest.json).
    /// - openclaw skills come from the openhuman workspace skills directory (SKILL.md/TOML).
    pub async fn list_all(&self) -> Vec<UnifiedSkillEntry> {
        let mut entries = Vec::new();

        // --- openhuman skills (QuickJS runtime) ---
        if let Ok(manifests) = self.engine.discover_skills().await {
            let snapshots = self.engine.list_skills();

            for manifest in &manifests {
                let tools = snapshots
                    .iter()
                    .find(|s| s.skill_id == manifest.id)
                    .map(|s| s.tools.clone())
                    .unwrap_or_default();

                entries.push(UnifiedSkillEntry {
                    id: manifest.id.clone(),
                    name: manifest.name.clone(),
                    skill_type: manifest.skill_type.clone(),
                    version: manifest.version.clone().unwrap_or_else(|| "0.1.0".to_string()),
                    description: manifest.description.clone().unwrap_or_default(),
                    tools,
                    setup: manifest.setup.clone(),
                });
            }
        }

        // --- openclaw skills (SKILL.md / SKILL.toml) ---
        let workspace_dir = workspace_dir();
        let openclaw_skills = load_skills(&workspace_dir);

        for skill in &openclaw_skills {
            let id = skill_to_id(skill);
            // Skip if already listed by the openhuman runtime (avoid duplicates).
            if entries.iter().any(|e| e.id == id) {
                continue;
            }

            let tools: Vec<ToolDefinition> = skill
                .tools
                .iter()
                .map(|t| ToolDefinition {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    input_schema: serde_json::json!({
                        "type": "object",
                        "properties": {}
                    }),
                })
                .collect();

            entries.push(UnifiedSkillEntry {
                id,
                name: skill.name.clone(),
                skill_type: "openclaw".to_string(),
                version: skill.version.clone(),
                description: skill.description.clone(),
                tools,
                setup: None,
            });
        }

        entries
    }

    /// Execute a skill by ID. Dispatches to the appropriate backend based on skill_type.
    pub async fn execute(
        &self,
        skill_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<UnifiedSkillResult, String> {
        let all = self.list_all().await;
        let entry = all
            .iter()
            .find(|e| e.id == skill_id)
            .ok_or_else(|| format!("Skill '{skill_id}' not found in unified registry"))?;

        match entry.skill_type.as_str() {
            "openhuman" => self.execute_openhuman(skill_id, tool_name, args).await,
            "openclaw" => self.execute_openclaw(skill_id, tool_name, args).await,
            other => Err(format!("Unknown skill type: '{other}'")),
        }
    }

    /// Dispatch to QuickJS runtime for openhuman skills.
    async fn execute_openhuman(
        &self,
        skill_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<UnifiedSkillResult, String> {
        let tool_result = self.engine.call_tool(skill_id, tool_name, args).await?;
        Ok(UnifiedSkillResult {
            skill_id: skill_id.to_string(),
            tool_name: Some(tool_name.to_string()),
            content: tool_result.content,
            is_error: tool_result.is_error,
            executed_at: Utc::now().to_rfc3339(),
        })
    }

    /// Dispatch to openclaw executor for SKILL.md/TOML skills.
    async fn execute_openclaw(
        &self,
        skill_id: &str,
        tool_name: &str,
        args: serde_json::Value,
    ) -> Result<UnifiedSkillResult, String> {
        let workspace_dir = workspace_dir();
        let skills = load_skills(&workspace_dir);

        let skill = skills
            .iter()
            .find(|s| skill_to_id(s) == skill_id)
            .ok_or_else(|| format!("openclaw skill '{skill_id}' not found on disk"))?;

        openclaw_executor::execute(skill, skill_id, tool_name, args).await
    }

    /// Generate a new skill from a spec, write it to disk, and return its registry entry.
    pub async fn generate(&self, spec: GenerateSkillSpec) -> Result<UnifiedSkillEntry, String> {
        match spec.skill_type.as_str() {
            "openhuman" => {
                // Find the skills source directory from the engine.
                let skills_dir = self.engine.skills_source_dir()?;
                generator::generate_openhuman(&spec, &skills_dir).await?;

                // Rediscover so the new skill appears in subsequent list_all() calls.
                let _ = self.engine.discover_skills().await;

                // Start the skill in the QuickJS runtime so call_tool() can execute it.
                let id = sanitize_id(&spec.name);
                let _ = self.engine.start_skill(&id).await;
                Ok(UnifiedSkillEntry {
                    id,
                    name: spec.name,
                    skill_type: "openhuman".to_string(),
                    version: "1.0.0".to_string(),
                    description: spec.description,
                    tools: vec![],
                    setup: None,
                })
            }
            "openclaw" => {
                generator::generate_openclaw(&spec).await?;

                let id = sanitize_id(&spec.name);
                Ok(UnifiedSkillEntry {
                    id,
                    name: spec.name,
                    skill_type: "openclaw".to_string(),
                    version: "1.0.0".to_string(),
                    description: spec.description,
                    tools: vec![],
                    setup: None,
                })
            }
            other => Err(format!("Unknown skill_type: '{other}'. Use 'openhuman' or 'openclaw'.")),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Derive a stable registry ID from an openclaw Skill.
fn skill_to_id(skill: &Skill) -> String {
    sanitize_id(&skill.name)
}

/// Convert a display name to a lowercase hyphen-separated ID.
fn sanitize_id(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Returns `~/.openhuman/workspace` as the base for openclaw skills.
fn workspace_dir() -> PathBuf {
    UserDirs::new()
        .map(|d| d.home_dir().join(".openhuman").join("workspace"))
        .unwrap_or_else(|| PathBuf::from(".openhuman/workspace"))
}
