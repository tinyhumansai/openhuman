//! Phase guidance rendering, effective tool-scope resolution, and workflow
//! selection (auto-match over `when_to_use`).

use super::types::{ToolScope, Workflow};

/// Render one phase's rules as a compact guidance block, or `None` when the
/// phase is absent or carries no rules.
pub fn phase_guidance(workflow: &Workflow, phase: &str) -> Option<String> {
    let p = workflow.phases.get(phase)?;
    if p.rules.is_empty() {
        return None;
    }
    let mut out = format!("### {} — phase {}\n", workflow.name, phase);
    if let Some(desc) = p.description.as_deref() {
        if !desc.trim().is_empty() {
            out.push_str(desc.trim());
            out.push('\n');
        }
    }
    for rule in &p.rules {
        out.push_str(&format!("- {rule}\n"));
    }
    Some(out)
}

/// Resolve the effective tool scope for a phase: the phase override unioned
/// over the workflow-level default. `allow` is deduplicated; `deny` is the
/// union of both. Returns `None` when neither the phase nor the workflow
/// declares a scope.
pub fn effective_tool_scope(workflow: &Workflow, phase: &str) -> Option<ToolScope> {
    let phase_scope = workflow.phases.get(phase).and_then(|p| p.tools.as_ref());
    let wf_scope = workflow.tools.as_ref();

    if phase_scope.is_none() && wf_scope.is_none() {
        return None;
    }

    let mut allow: Vec<String> = Vec::new();
    let mut deny: Vec<String> = Vec::new();
    for scope in [wf_scope, phase_scope].into_iter().flatten() {
        for a in &scope.allow {
            if !allow.contains(a) {
                allow.push(a.clone());
            }
        }
        for d in &scope.deny {
            if !deny.contains(d) {
                deny.push(d.clone());
            }
        }
    }
    Some(ToolScope { allow, deny })
}

/// Best-match a workflow against a free-text query by scoring word overlap
/// against each workflow's `when_to_use`. Returns `None` when nothing overlaps.
pub fn best_match<'a>(workflows: &'a [Workflow], query: &str) -> Option<&'a Workflow> {
    let query_words = tokenize(query);
    if query_words.is_empty() {
        return None;
    }
    let mut best: Option<(&Workflow, usize)> = None;
    for wf in workflows {
        if wf.when_to_use.trim().is_empty() {
            continue;
        }
        let hint_words = tokenize(&wf.when_to_use);
        let score = hint_words
            .iter()
            .filter(|w| query_words.contains(*w))
            .count();
        if score == 0 {
            continue;
        }
        match best {
            Some((_, best_score)) if best_score >= score => {}
            _ => best = Some((wf, score)),
        }
    }
    best.map(|(wf, _)| wf)
}

/// Lowercase word tokens of length ≥ 3 (drops stop-word-ish short tokens).
fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_string())
        .collect()
}

#[cfg(test)]
#[path = "select_tests.rs"]
mod tests;
