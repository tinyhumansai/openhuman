//! Prompt-injection helpers: render the available-workflows catalog.
//!
//! Analogous to `render_available_skills()` — produces an XML-ish catalog
//! (id + name + when_to_use) the agent reads to decide which workflow to load
//! for a task.

use super::types::Workflow;

/// Render the available-workflows catalog. Returns an empty string when there
/// are no workflows (so callers can unconditionally concatenate the result).
pub fn render_workflow_catalog(workflows: &[Workflow]) -> String {
    if workflows.is_empty() {
        return String::new();
    }
    let mut out = String::from("<available_workflows>\n");
    for wf in workflows {
        out.push_str(&format!(
            "  <workflow id=\"{}\" name=\"{}\">\n    {}\n  </workflow>\n",
            xml_escape(&wf.dir_name),
            xml_escape(&wf.name),
            xml_escape(&wf.when_to_use),
        ));
    }
    out.push_str("</available_workflows>\n");
    out
}

/// Alias used by the prompt builder; identical to [`render_workflow_catalog`].
pub fn render_available_workflows(workflows: &[Workflow]) -> String {
    render_workflow_catalog(workflows)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
#[path = "inject_tests.rs"]
mod tests;
