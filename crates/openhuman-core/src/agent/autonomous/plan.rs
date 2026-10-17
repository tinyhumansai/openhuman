//! Durable project decomposition and dependency invalidation. The host owns
//! state transitions; model-authored plans and evidence are not proof of quality.
use super::types::{Phase, Work};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    Working,
    Verified,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub parent: Option<String>,
    pub title: String,
    pub acceptance_criteria: Vec<String>,
    pub depends_on: Vec<String>,
    pub state: TaskState,
    pub evidence: Vec<String>,
    pub revision: u64,
}
fn index(work: &Work, id: &str) -> Result<usize> {
    work.plan
        .iter()
        .position(|t| t.id == id)
        .ok_or_else(|| anyhow::anyhow!("Unknown task {id}"))
}
pub fn all_verified(work: &Work) -> bool {
    work.plan.iter().all(|t| t.state == TaskState::Verified)
}
fn children_verified(work: &Work, id: &str) -> bool {
    work.plan
        .iter()
        .filter(|t| t.parent.as_deref() == Some(id))
        .all(|t| t.state == TaskState::Verified)
}
fn dependencies_verified(work: &Work, task: &Task) -> bool {
    task.depends_on.iter().all(|id| {
        work.plan
            .iter()
            .any(|t| &t.id == id && t.state == TaskState::Verified)
    })
}
/// Additive planning only: completed requirements cannot silently disappear.
pub fn add(work: &mut Work, task: Task) -> Result<()> {
    ensure!(
        work.plan.len() < 2048,
        "Split this project into separate autonomous goals (2048 tasks per goal)"
    );
    ensure!(
        !task.id.is_empty()
            && task.id.len() <= 96
            && task
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')),
        "task_id must be 1–96 ASCII letters, digits, dots, underscores or hyphens"
    );
    ensure!(
        !work.plan.iter().any(|t| t.id == task.id),
        "Task already exists; reopen it instead of replacing requirements"
    );
    ensure!(
        !task.title.trim().is_empty() && !task.acceptance_criteria.is_empty(),
        "Each task needs a title and acceptance criteria"
    );
    let mut ancestors = HashSet::new();
    let mut parent = task.parent.as_deref();
    while let Some(id) = parent {
        ensure!(ancestors.insert(id.to_owned()), "Cyclic parent chain");
        let node = &work.plan[index(work, id)?];
        parent = node.parent.as_deref();
    }
    ensure!(
        ancestors.len() < 10,
        "Plan supports up to ten nested levels; split this work package"
    );
    for dependency in &task.depends_on {
        index(work, dependency)?;
        ensure!(
            !ancestors.contains(dependency),
            "A child cannot depend on its ancestor's completion"
        );
    }
    // Existing tasks alone may be dependencies. A newly inserted node cannot
    // create a dependency cycle; later revisions never rewrite dependency edges.
    validate_dag(work, &task)?;
    work.plan.push(task);
    for ancestor in ancestors {
        invalidate_scope(work, &ancestor, false)?;
    }
    cancel_review(work);
    Ok(())
}
fn validate_dag(work: &Work, added: &Task) -> Result<()> {
    use std::collections::HashMap;
    let tasks: Vec<_> = work.plan.iter().chain(std::iter::once(added)).collect();
    let mut remaining: HashMap<&str, usize> = tasks.iter().map(|t| (t.id.as_str(), 0)).collect();
    let mut consumers: HashMap<&str, Vec<&str>> = HashMap::new();
    for task in &tasks {
        for dep in &task.depends_on {
            *remaining.get_mut(task.id.as_str()).unwrap() += 1;
            consumers.entry(dep).or_default().push(&task.id);
        }
        if let Some(parent) = &task.parent {
            *remaining.get_mut(parent.as_str()).unwrap() += 1;
            consumers.entry(&task.id).or_default().push(parent);
        }
    }
    let mut ready: Vec<_> = remaining
        .iter()
        .filter_map(|(&id, &n)| (n == 0).then_some(id))
        .collect();
    let mut visited = 0;
    while let Some(id) = ready.pop() {
        visited += 1;
        for consumer in consumers.get(id).into_iter().flatten() {
            let count = remaining.get_mut(consumer).unwrap();
            *count -= 1;
            if *count == 0 {
                ready.push(consumer);
            }
        }
    }
    ensure!(
        visited == tasks.len(),
        "Dependencies and component hierarchy would create a cycle"
    );
    Ok(())
}
pub fn begin(work: &mut Work, id: &str) -> Result<()> {
    let i = index(work, id)?;
    ensure!(
        dependencies_verified(work, &work.plan[i]),
        "Complete and verify dependencies first"
    );
    ensure!(
        work.plan[i].state != TaskState::Verified,
        "Reopen a verified task before changing it"
    );
    work.plan[i].state = TaskState::Working;
    cancel_review(work);
    Ok(())
}
pub fn request_review(work: &mut Work, id: &str) -> Result<()> {
    let i = index(work, id)?;
    ensure!(
        work.plan[i].state == TaskState::Working,
        "Start this task and implement it before review"
    );
    ensure!(
        dependencies_verified(work, &work.plan[i]) && children_verified(work, id),
        "Verify dependencies and child tasks before reviewing this component"
    );
    work.review_task = Some(id.into());
    work.phase = Phase::Reviewing;
    work.finish_requested = false;
    work.evidence.clear();
    Ok(())
}
/// Changing a component invalidates ancestors and consumers transitively.
pub fn invalidate(work: &mut Work, id: &str) -> Result<()> {
    invalidate_scope(work, id, true)
}
fn invalidate_scope(work: &mut Work, id: &str, changed_contract: bool) -> Result<()> {
    index(work, id)?;
    let mut dirty = HashSet::from([id.to_owned()]);
    let mut changed = if changed_contract {
        dirty.clone()
    } else {
        HashSet::new()
    };
    loop {
        let before = (dirty.len(), changed.len());
        for task in &work.plan {
            // A changed parent contract affects its own descendants. Merely
            // needing an integration recheck does not invalidate siblings.
            if task.parent.as_ref().is_some_and(|p| changed.contains(p))
                || task.depends_on.iter().any(|dep| dirty.contains(dep))
            {
                changed.insert(task.id.clone());
                dirty.insert(task.id.clone());
            }
            if dirty.contains(&task.id) {
                if let Some(parent) = &task.parent {
                    dirty.insert(parent.clone());
                }
            }
        }
        if before == (dirty.len(), changed.len()) {
            break;
        }
    }
    for task in &mut work.plan {
        if dirty.contains(&task.id) {
            task.state = TaskState::Pending;
            task.evidence.clear();
            task.revision = task.revision.saturating_add(1);
        }
    }
    cancel_review(work);
    Ok(())
}
fn cancel_review(work: &mut Work) {
    work.review_task = None;
    work.finish_requested = false;
    work.evidence.clear();
    work.phase = Phase::Working;
}
pub fn criteria(work: &Work) -> Result<&[String]> {
    match &work.review_task {
        Some(id) => Ok(&work.plan[index(work, id)?].acceptance_criteria),
        None => Ok(&work.acceptance_criteria),
    }
}
/// Called only by the host after a separate slice observed successful checks.
pub fn accept_review(work: &mut Work) -> Result<()> {
    if let Some(id) = work.review_task.clone() {
        let i = index(work, &id)?;
        ensure!(
            dependencies_verified(work, &work.plan[i]) && children_verified(work, &id),
            "Review dependencies changed"
        );
        work.plan[i].state = TaskState::Verified;
        work.plan[i].evidence = work.evidence.clone();
        cancel_review(work);
        work.last_error = None;
        work.worker_reports.clear();
    } else {
        ensure!(
            all_verified(work),
            "Project still contains unverified work packages"
        );
        work.phase = Phase::Complete;
        work.last_error = None;
        work.worker_reports.clear();
    }
    Ok(())
}
/// Keep continuation context bounded even for a large plan. Details live on disk.
pub fn context(work: &Work) -> String {
    let verified = work
        .plan
        .iter()
        .filter(|t| t.state == TaskState::Verified)
        .count();
    let current = work
        .review_task
        .as_deref()
        .and_then(|id| work.plan.iter().find(|t| t.id == id));
    let ready: Vec<_> = work
        .plan
        .iter()
        .filter(|t| {
            t.state != TaskState::Verified
                && dependencies_verified(work, t)
                && children_verified(work, &t.id)
        })
        .take(8)
        .collect();
    format!(
        "\nProject plan: {verified}/{} verified. Review target: {}. Ready work packages: {}. Use plan_task to decompose complex components, task_start, task_review and task_reopen. Each package needs real implementation and a separate host review; finish never verifies an entire project while packages remain. Persist design decisions, references, measurements and visual evidence in project Markdown files.\n",
        work.plan.len(),
        serde_json::to_string(&current).unwrap_or_default(),
        serde_json::to_string(&ready).unwrap_or_default()
    )
}
/// Page the plan so weak/small-context models never receive thousands of
/// work packages on every checkpoint. `status` accepts offset or task_id.
pub fn report(work: &Work, args: &serde_json::Value) -> serde_json::Value {
    let mut result = serde_json::to_value(work).unwrap_or_default();
    let offset = args["offset"]
        .as_u64()
        .unwrap_or(0)
        .min(work.plan.len() as u64) as usize;
    let page: Vec<_> = work.plan.iter().skip(offset).take(16).collect();
    result["plan"] = serde_json::json!(page);
    result["plan_total"] = serde_json::json!(work.plan.len());
    result["plan_next_offset"] = if offset + page.len() < work.plan.len() {
        serde_json::json!(offset + page.len())
    } else {
        serde_json::Value::Null
    };
    if let Some(id) = args["task_id"].as_str() {
        result["selected_task"] = serde_json::json!(work.plan.iter().find(|t| t.id == id));
    }
    result
}
#[cfg(test)]
#[path = "plan_tests.rs"]
mod tests;
