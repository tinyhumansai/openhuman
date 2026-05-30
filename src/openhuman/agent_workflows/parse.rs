//! WORKFLOW.md frontmatter parsing.
//!
//! Mirrors `skills::ops_parse`: split a leading `---`-delimited YAML block from
//! the markdown body, deserialize the frontmatter, and collect non-fatal
//! warnings (rather than failing) when the YAML is malformed.

use std::path::Path;

use super::types::WorkflowFrontmatter;

/// Parse a `WORKFLOW.md` file into `(frontmatter, body, warnings)`.
///
/// Returns `None` when the file cannot be read or the frontmatter block is
/// opened with `---` but never terminated.
pub fn parse_workflow_md(path: &Path) -> Option<(WorkflowFrontmatter, String, Vec<String>)> {
    let content = std::fs::read_to_string(path).ok()?;
    parse_workflow_md_str(&content)
}

/// Content-only variant of [`parse_workflow_md`].
pub fn parse_workflow_md_str(content: &str) -> Option<(WorkflowFrontmatter, String, Vec<String>)> {
    let mut lines = content.lines();
    let first = lines.next()?;
    if first.trim() != "---" {
        // No frontmatter — treat the whole file as body.
        return Some((
            WorkflowFrontmatter::default(),
            content.to_string(),
            Vec::new(),
        ));
    }

    let mut yaml = String::new();
    let mut terminated = false;
    let mut body = String::new();
    for line in lines {
        if !terminated && line.trim() == "---" {
            terminated = true;
            continue;
        }
        if !terminated {
            yaml.push_str(line);
            yaml.push('\n');
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }

    if !terminated {
        return None;
    }

    let mut warnings = Vec::new();
    let frontmatter = match serde_yaml::from_str::<WorkflowFrontmatter>(&yaml) {
        Ok(fm) => fm,
        Err(err) => {
            log::warn!("[workflows] failed to parse frontmatter: {err}");
            warnings.push(format!("frontmatter parse error: {err}"));
            WorkflowFrontmatter::default()
        }
    };

    Some((frontmatter, body, warnings))
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
