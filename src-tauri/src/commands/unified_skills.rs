//! Tauri commands for the unified skill registry.
//!
//! These commands expose a single, type-agnostic API to the frontend WebView.
//! Internally they dispatch to the QuickJS runtime (openhuman skills) or the
//! file-based executor (openclaw skills) based on `skill_type`.
//!
//! Commands are desktop-only in this host.

use crate::runtime::types::{UnifiedSkillEntry, UnifiedSkillResult};
use crate::unified_skills::self_evolve::{SelfEvolveRequest, SelfEvolveResult};
use crate::unified_skills::GenerateSkillSpec;
use crate::{runtime::qjs_engine::RuntimeEngine, unified_skills::UnifiedSkillRegistry};
use std::sync::Arc;
use tauri::State;

/// List all skills from the unified registry (both openhuman and openclaw types).
#[tauri::command]
pub async fn unified_list_skills(
    engine: State<'_, Arc<RuntimeEngine>>,
) -> Result<Vec<UnifiedSkillEntry>, String> {
    let registry = UnifiedSkillRegistry::new(Arc::clone(&engine));
    Ok(registry.list_all().await)
}

/// Execute a named tool on any registered skill.
///
/// Dispatches based on skill_type:
/// - `openhuman` → QuickJS runtime
/// - `openclaw`   → shell/http executor or returns prompt content
#[tauri::command]
pub async fn unified_execute_skill(
    engine: State<'_, Arc<RuntimeEngine>>,
    skill_id: String,
    tool_name: String,
    args: serde_json::Value,
) -> Result<UnifiedSkillResult, String> {
    let registry = UnifiedSkillRegistry::new(Arc::clone(&engine));
    registry.execute(&skill_id, &tool_name, args).await
}

/// Programmatically generate a new skill, register it, and return its entry.
///
/// For `skill_type = "openhuman"`: writes manifest.json + index.js to the skills dir.
/// For `skill_type = "openclaw"`:   writes SKILL.md or SKILL.toml to workspace/skills/.
///
/// The skill is immediately available in subsequent `unified_list_skills` calls.
#[tauri::command]
pub async fn unified_generate_skill(
    engine: State<'_, Arc<RuntimeEngine>>,
    spec: GenerateSkillSpec,
) -> Result<UnifiedSkillEntry, String> {
    let registry = UnifiedSkillRegistry::new(Arc::clone(&engine));
    registry.generate(spec).await
}

/// Self-evolving skill generation.
///
/// Uses an LLM to generate QuickJS skill code, tests it in an isolated
/// QuickJS context, and iterates until the skill passes or the iteration
/// budget is exhausted. Emits `skill:evolve:progress` events after each
/// iteration.
#[tauri::command]
pub async fn unified_self_evolve_skill(
    engine: State<'_, Arc<RuntimeEngine>>,
    app: tauri::AppHandle,
    request: SelfEvolveRequest,
) -> Result<SelfEvolveResult, String> {
    use crate::unified_skills::self_evolve::SkillEvolver;
    use tauri::Emitter;

    let registry = Arc::new(UnifiedSkillRegistry::new(Arc::clone(&engine)));
    let evolver = SkillEvolver::new(registry);
    let app_clone = app.clone();

    evolver
        .evolve(request, move |iteration, passed| {
            let _ = app_clone.emit(
                "skill:evolve:progress",
                serde_json::json!({
                    "iteration": iteration,
                    "passed": passed,
                }),
            );
        })
        .await
}
