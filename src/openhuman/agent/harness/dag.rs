//! Directed Acyclic Graph (DAG) for task planning.
//!
//! The Planner archetype produces a `TaskDag` that the Orchestrator executes
//! level-by-level. Nodes with satisfied dependencies run concurrently within
//! a level.

use super::archetypes::AgentArchetype;
use super::types::{SubAgentResult, TaskId, TaskStatus};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

/// A single task node in the execution DAG.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    /// Unique identifier within this DAG.
    pub id: TaskId,
    /// Human-readable description of what this task does.
    pub description: String,
    /// Which archetype should execute this task.
    pub archetype: AgentArchetype,
    /// Task IDs that must complete before this node can run.
    #[serde(default)]
    pub depends_on: Vec<TaskId>,
    /// Acceptance criteria — how the Orchestrator judges success.
    #[serde(default)]
    pub acceptance_criteria: String,
    /// Current execution status.
    #[serde(default)]
    pub status: TaskStatus,
    /// Result from the sub-agent, populated after execution.
    #[serde(skip)]
    pub result: Option<SubAgentResult>,
    /// Number of retry attempts made.
    #[serde(default)]
    pub retry_count: u8,
}

/// The full task DAG produced by the Planner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskDag {
    /// The user's original goal.
    pub root_goal: String,
    /// Task nodes in insertion order.
    pub nodes: Vec<TaskNode>,
}

impl TaskDag {
    /// Create a new DAG with a single direct-execution node (bypass planning overhead).
    pub fn single_task(goal: &str, archetype: AgentArchetype, description: &str) -> Self {
        Self {
            root_goal: goal.to_string(),
            nodes: vec![TaskNode {
                id: "task-1".to_string(),
                description: description.to_string(),
                archetype,
                depends_on: Vec::new(),
                acceptance_criteria: String::new(),
                status: TaskStatus::Pending,
                result: None,
                retry_count: 0,
            }],
        }
    }

    /// Validate the DAG: check for missing dependencies and cycles.
    pub fn validate(&self) -> Result<(), DagError> {
        let ids: HashSet<&str> = self.nodes.iter().map(|n| n.id.as_str()).collect();

        // Check all dependency references exist.
        for node in &self.nodes {
            for dep in &node.depends_on {
                if !ids.contains(dep.as_str()) {
                    return Err(DagError::MissingDependency {
                        node: node.id.clone(),
                        missing: dep.clone(),
                    });
                }
            }
            // Self-dependency check.
            if node.depends_on.contains(&node.id) {
                return Err(DagError::Cycle);
            }
        }

        // Full cycle detection via Kahn's algorithm.
        if self.topological_sort().is_none() {
            return Err(DagError::Cycle);
        }

        Ok(())
    }

    /// Topological sort using Kahn's algorithm.
    /// Returns `None` if the graph contains a cycle.
    pub fn topological_sort(&self) -> Option<Vec<&TaskId>> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut adj: HashMap<&str, Vec<&str>> = HashMap::new();

        for node in &self.nodes {
            in_degree.entry(node.id.as_str()).or_insert(0);
            adj.entry(node.id.as_str()).or_default();
            for dep in &node.depends_on {
                adj.entry(dep.as_str()).or_default().push(node.id.as_str());
                *in_degree.entry(node.id.as_str()).or_insert(0) += 1;
            }
        }

        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut sorted = Vec::new();
        while let Some(id) = queue.pop_front() {
            sorted.push(id);
            if let Some(dependents) = adj.get(id) {
                for &dep in dependents {
                    if let Some(deg) = in_degree.get_mut(dep) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(dep);
                        }
                    }
                }
            }
        }

        if sorted.len() != self.nodes.len() {
            return None; // cycle detected
        }

        // Map back to TaskId references.
        let id_map: HashMap<&str, &TaskId> =
            self.nodes.iter().map(|n| (n.id.as_str(), &n.id)).collect();

        Some(
            sorted
                .into_iter()
                .filter_map(|s| id_map.get(s).copied())
                .collect(),
        )
    }

    /// Return execution levels: groups of task IDs that can run concurrently.
    /// Each level contains only tasks whose dependencies are in earlier levels.
    pub fn execution_levels(&self) -> Vec<Vec<&TaskId>> {
        let mut remaining: HashMap<&str, HashSet<&str>> = self
            .nodes
            .iter()
            .map(|n| {
                let deps: HashSet<&str> = n.depends_on.iter().map(|d| d.as_str()).collect();
                (n.id.as_str(), deps)
            })
            .collect();

        let mut levels = Vec::new();
        let mut completed: HashSet<&str> = HashSet::new();

        while !remaining.is_empty() {
            let ready: Vec<&str> = remaining
                .iter()
                .filter(|(_, deps)| deps.iter().all(|d| completed.contains(d)))
                .map(|(&id, _)| id)
                .collect();

            if ready.is_empty() {
                // Remaining nodes have unsatisfied deps (should be caught by validate).
                break;
            }

            let id_map: HashMap<&str, &TaskId> =
                self.nodes.iter().map(|n| (n.id.as_str(), &n.id)).collect();

            let level: Vec<&TaskId> = ready
                .iter()
                .filter_map(|&id| id_map.get(id).copied())
                .collect();

            for &id in &ready {
                remaining.remove(id);
                completed.insert(id);
            }

            levels.push(level);
        }

        levels
    }

    /// Find a node by ID.
    pub fn node(&self, id: &str) -> Option<&TaskNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// Find a mutable node by ID.
    pub fn node_mut(&mut self, id: &str) -> Option<&mut TaskNode> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    /// Number of nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the DAG is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Whether all nodes are completed or cancelled.
    pub fn is_finished(&self) -> bool {
        self.nodes
            .iter()
            .all(|n| matches!(n.status, TaskStatus::Completed | TaskStatus::Cancelled))
    }

    /// Collect all completed results.
    pub fn completed_results(&self) -> Vec<&SubAgentResult> {
        self.nodes
            .iter()
            .filter_map(|n| n.result.as_ref())
            .collect()
    }
}

/// Errors during DAG validation.
#[derive(Debug, thiserror::Error)]
pub enum DagError {
    #[error("cycle detected in task DAG")]
    Cycle,
    #[error("node {node} depends on missing node {missing}")]
    MissingDependency { node: String, missing: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_node(id: &str, deps: &[&str]) -> TaskNode {
        TaskNode {
            id: id.to_string(),
            description: format!("Task {id}"),
            archetype: AgentArchetype::CodeExecutor,
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            acceptance_criteria: String::new(),
            status: TaskStatus::Pending,
            result: None,
            retry_count: 0,
        }
    }

    #[test]
    fn single_task_bypasses_dag() {
        let dag = TaskDag::single_task("do thing", AgentArchetype::Researcher, "research it");
        assert_eq!(dag.len(), 1);
        assert!(dag.validate().is_ok());
        let levels = dag.execution_levels();
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].len(), 1);
    }

    #[test]
    fn linear_chain() {
        let dag = TaskDag {
            root_goal: "build feature".into(),
            nodes: vec![
                make_node("a", &[]),
                make_node("b", &["a"]),
                make_node("c", &["b"]),
            ],
        };
        assert!(dag.validate().is_ok());
        let levels = dag.execution_levels();
        assert_eq!(levels.len(), 3);
    }

    #[test]
    fn parallel_then_join() {
        let dag = TaskDag {
            root_goal: "parallel work".into(),
            nodes: vec![
                make_node("a", &[]),
                make_node("b", &[]),
                make_node("c", &["a", "b"]),
            ],
        };
        assert!(dag.validate().is_ok());
        let levels = dag.execution_levels();
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[0].len(), 2); // a and b in parallel
        assert_eq!(levels[1].len(), 1); // c after both
    }

    #[test]
    fn cycle_detection() {
        let dag = TaskDag {
            root_goal: "cycle".into(),
            nodes: vec![make_node("a", &["b"]), make_node("b", &["a"])],
        };
        assert!(matches!(dag.validate(), Err(DagError::Cycle)));
    }

    #[test]
    fn missing_dependency() {
        let dag = TaskDag {
            root_goal: "missing".into(),
            nodes: vec![make_node("a", &["nonexistent"])],
        };
        assert!(matches!(
            dag.validate(),
            Err(DagError::MissingDependency { .. })
        ));
    }

    #[test]
    fn self_dependency() {
        let dag = TaskDag {
            root_goal: "self".into(),
            nodes: vec![make_node("a", &["a"])],
        };
        assert!(dag.validate().is_err());
    }

    #[test]
    fn topological_sort_order() {
        let dag = TaskDag {
            root_goal: "order".into(),
            nodes: vec![
                make_node("c", &["a", "b"]),
                make_node("a", &[]),
                make_node("b", &["a"]),
            ],
        };
        let sorted = dag.topological_sort().unwrap();
        let pos: HashMap<&str, usize> = sorted
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        assert!(pos["a"] < pos["b"]);
        assert!(pos["b"] < pos["c"]);
    }
}
