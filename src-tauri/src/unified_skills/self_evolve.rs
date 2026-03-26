//! Self-evolving skill orchestrator.
//!
//! Drives a generate → test → fix loop that uses an LLM to produce QuickJS
//! skill code, validates it in an isolated context, and iterates until the
//! skill passes or the iteration budget is exhausted.

use crate::runtime::types::UnifiedSkillResult;
use crate::unified_skills::llm_generator::LlmGenerator;
use crate::unified_skills::skill_tester::SkillTester;
use crate::unified_skills::{generator, GenerateSkillSpec, UnifiedSkillRegistry};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Public request / response types
// ---------------------------------------------------------------------------

/// Request payload for `unified_self_evolve_skill`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfEvolveRequest {
    /// Natural language description of what the skill should do.
    pub task_description: String,
    /// Maximum LLM-generate-test iterations (default: 3).
    pub max_iterations: Option<u32>,
    /// Wall-clock timeout in seconds for the whole loop (default: 120).
    pub timeout_secs: Option<u64>,
    /// Anthropic API key.  Falls back to `ANTHROPIC_API_KEY` env var when absent.
    pub anthropic_api_key: Option<String>,
}

/// Per-iteration audit record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterationLog {
    pub iteration: u32,
    pub generated_code: String,
    pub test_output: String,
    pub passed: bool,
    pub error: Option<String>,
}

/// Final result of the evolve loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfEvolveResult {
    pub skill_id: String,
    pub success: bool,
    pub iterations_used: u32,
    pub audit_log: Vec<IterationLog>,
    pub files_created: Vec<String>,
    pub final_result: Option<UnifiedSkillResult>,
    pub failure_reason: Option<String>,
}

// ---------------------------------------------------------------------------
// SkillEvolver
// ---------------------------------------------------------------------------

/// Orchestrates the generate-test-fix loop.
pub struct SkillEvolver {
    pub registry: Arc<UnifiedSkillRegistry>,
}

impl SkillEvolver {
    pub fn new(registry: Arc<UnifiedSkillRegistry>) -> Self {
        Self { registry }
    }

    /// Run the self-evolution loop.
    ///
    /// `on_progress` is called after each iteration with `(iteration_index, passed)`.
    pub async fn evolve(
        &self,
        req: SelfEvolveRequest,
        on_progress: impl Fn(u32, bool) + Send + Sync + 'static,
    ) -> Result<SelfEvolveResult, String> {
        let max_iter = req.max_iterations.unwrap_or(3);
        let timeout_secs = req.timeout_secs.unwrap_or(120);

        let api_key = req
            .anthropic_api_key
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
            .ok_or_else(|| {
                "No Anthropic API key configured. Set ANTHROPIC_API_KEY or pass anthropic_api_key in the request.".to_string()
            })?;

        let task_description = req.task_description.clone();
        let registry = Arc::clone(&self.registry);

        let loop_result = tokio::time::timeout(
            std::time::Duration::from_secs(timeout_secs),
            Self::run_loop(
                registry,
                api_key,
                task_description,
                max_iter,
                on_progress,
            ),
        )
        .await;

        match loop_result {
            Ok(inner) => inner,
            Err(_elapsed) => Err("Self-evolve timed out".to_string()),
        }
    }

    // -----------------------------------------------------------------------
    // Private
    // -----------------------------------------------------------------------

    async fn run_loop(
        registry: Arc<UnifiedSkillRegistry>,
        api_key: String,
        task_description: String,
        max_iter: u32,
        on_progress: impl Fn(u32, bool) + Send + Sync + 'static,
    ) -> Result<SelfEvolveResult, String> {
        let generator_llm = LlmGenerator::new(api_key);

        let mut audit_log: Vec<IterationLog> = Vec::new();
        let mut files_created: Vec<String> = Vec::new();
        let mut last_error = String::new();
        let mut last_spec: Option<GenerateSkillSpec> = None;
        let mut skill_id = String::new();

        let skills_dir = registry.skills_dir()?;
        let mut success = false;

        for i in 0..max_iter {
            // -- Generate --
            let spec = if i == 0 {
                generator_llm
                    .generate_spec(&task_description)
                    .await
                    .map_err(|e| format!("LLM generation failed (iter {i}): {e}"))?
            } else {
                let prev_code = last_spec
                    .as_ref()
                    .and_then(|s| s.full_index_js.clone())
                    .or_else(|| {
                        last_spec
                            .as_ref()
                            .and_then(|s| s.tool_code.clone())
                    })
                    .unwrap_or_default();

                generator_llm
                    .fix_spec(&task_description, &prev_code, &last_error)
                    .await
                    .map_err(|e| format!("LLM fix failed (iter {i}): {e}"))?
            };

            // Derive skill id from the spec name.
            skill_id = sanitize_id(&spec.name);

            // -- Write files to disk --
            let written = generator::generate_openhuman(&spec, &skills_dir)
                .await
                .map_err(|e| format!("File generation failed (iter {i}): {e}"))?;

            files_created = written
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();

            // -- Test in isolation --
            let skill_dir = skills_dir.join(&skill_id);
            let test = SkillTester::run_isolated(&skill_dir).await;

            let generated_code = spec
                .full_index_js
                .clone()
                .or_else(|| spec.tool_code.clone())
                .unwrap_or_default();

            audit_log.push(IterationLog {
                iteration: i,
                generated_code: generated_code.clone(),
                test_output: test.output.clone(),
                passed: test.passed,
                error: test.error.clone(),
            });

            on_progress(i, test.passed);

            last_spec = Some(spec);

            if test.passed {
                success = true;
                break;
            }

            last_error = test
                .error
                .clone()
                .unwrap_or_else(|| "Unknown test error".to_string());

            // Clean up the failed attempt so a fresh attempt starts clean.
            let _ = tokio::fs::remove_dir_all(&skill_dir).await;
            files_created.clear();
        }

        if !success {
            return Ok(SelfEvolveResult {
                skill_id,
                success: false,
                iterations_used: audit_log.len() as u32,
                audit_log,
                files_created,
                final_result: None,
                failure_reason: Some(last_error),
            });
        }

        // -- Register and start the new skill --
        let _ = registry.engine().discover_skills().await;
        let _ = registry.engine().start_skill(&skill_id).await; // best-effort

        // -- Execute the skill's first tool --
        let skills = registry.list_all().await;
        let tool_name = skills
            .iter()
            .find(|s| s.id == skill_id)
            .and_then(|s| s.tools.first())
            .map(|t| t.name.clone())
            .unwrap_or_default();

        let final_result = if !tool_name.is_empty() {
            registry
                .execute(&skill_id, &tool_name, serde_json::json!({}))
                .await
                .ok()
        } else {
            None
        };

        Ok(SelfEvolveResult {
            skill_id,
            success: true,
            iterations_used: audit_log.len() as u32,
            audit_log,
            files_created,
            final_result,
            failure_reason: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
