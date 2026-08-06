//! One-time migration from OpenHuman's retired file-backed thread-goal store
//! into tinyagents' authoritative `graph.goals` namespace.

use std::path::Path;
use std::sync::Arc;

use ::tinyagents::graph::goals::store::GOALS_NAMESPACE;
use ::tinyagents::harness::store::Store;

use super::ThreadGoal;
use crate::openhuman::agent::session_import::ops::open_session_stores;

const LEGACY_GOALS_DIR: &str = "thread_goals";
const LEGACY_GOALS_EXTENSION: &str = "json";

pub(crate) fn goals_store(workspace_dir: &Path) -> Arc<dyn Store> {
    Arc::new(open_session_stores(workspace_dir).kv)
}

fn goal_key(thread_id: &str) -> String {
    thread_id
        .trim()
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn legacy_goal_path(workspace_dir: &Path, thread_id: &str) -> Result<std::path::PathBuf, String> {
    let thread_id = thread_id.trim();
    if thread_id.is_empty() {
        return Err("invalid thread goal thread_id: empty or whitespace".to_string());
    }
    Ok(workspace_dir.join(LEGACY_GOALS_DIR).join(format!(
        "{}.{LEGACY_GOALS_EXTENSION}",
        hex::encode(thread_id.as_bytes())
    )))
}

pub(crate) async fn delete_legacy_goal_file(
    workspace_dir: &Path,
    thread_id: &str,
) -> Result<bool, String> {
    let path = legacy_goal_path(workspace_dir, thread_id)?;
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "delete legacy thread goal {}: {error}",
            path.display()
        )),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GoalMigrationReport {
    pub total: usize,
    pub copied: usize,
    pub skipped: usize,
}

struct LegacyGoalRow {
    path: std::path::PathBuf,
    goal: ThreadGoal,
}

async fn read_legacy_goals(workspace_dir: &Path) -> Result<Vec<LegacyGoalRow>, String> {
    let dir = workspace_dir.join(LEGACY_GOALS_DIR);
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "read legacy thread goals dir {}: {error}",
                dir.display()
            ))
        }
    };
    let mut goals = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|error| format!("iterate legacy thread goals dir: {error}"))?
    {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some(LEGACY_GOALS_EXTENSION)
        {
            continue;
        }
        if let Ok(body) = tokio::fs::read_to_string(&path).await {
            if let Ok(goal) = serde_json::from_str::<ThreadGoal>(&body) {
                goals.push(LegacyGoalRow { path, goal });
            }
        }
    }
    Ok(goals)
}

pub async fn migrate_legacy_goals(workspace_dir: &Path) -> Result<GoalMigrationReport, String> {
    let legacy = read_legacy_goals(workspace_dir).await?;
    let store = goals_store(workspace_dir);
    let mut report = GoalMigrationReport {
        total: legacy.len(),
        ..Default::default()
    };

    for row in legacy {
        let key = goal_key(&row.goal.thread_id);
        if store
            .get(GOALS_NAMESPACE, &key)
            .await
            .map_err(|error| format!("read tinyagents goal during migration: {error}"))?
            .is_some()
        {
            report.skipped += 1;
        } else {
            let value = serde_json::to_value(&row.goal)
                .map_err(|error| format!("serialize legacy thread goal: {error}"))?;
            store
                .put(GOALS_NAMESPACE, &key, value)
                .await
                .map_err(|error| format!("write tinyagents goal during migration: {error}"))?;
            report.copied += 1;
        }
        tokio::fs::remove_file(&row.path)
            .await
            .map_err(|error| format!("remove migrated goal {}: {error}", row.path.display()))?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::tinyagents::graph::goals::{store, ThreadGoalStatus};

    #[tokio::test]
    async fn migrates_legacy_goal_into_tinyagents_store() {
        let temp = tempfile::tempdir().unwrap();
        let legacy_dir = temp.path().join(LEGACY_GOALS_DIR);
        tokio::fs::create_dir_all(&legacy_dir).await.unwrap();
        let goal = ThreadGoal {
            thread_id: "thread-1".into(),
            goal_id: "legacy-id".into(),
            objective: "legacy objective".into(),
            status: ThreadGoalStatus::Active,
            token_budget: Some(100),
            tokens_used: 10,
            time_used_seconds: 2,
            created_at_ms: 1,
            updated_at_ms: 2,
            continuation_suppressed: false,
        };
        let path = legacy_goal_path(temp.path(), &goal.thread_id).unwrap();
        tokio::fs::write(&path, serde_json::to_vec(&goal).unwrap())
            .await
            .unwrap();

        let report = migrate_legacy_goals(temp.path()).await.unwrap();
        assert_eq!(
            report,
            GoalMigrationReport {
                total: 1,
                copied: 1,
                skipped: 0
            }
        );
        assert_eq!(
            store::get(&goals_store(temp.path()), "thread-1")
                .await
                .unwrap()
                .unwrap(),
            goal
        );
        assert!(!path.exists());
    }
}
