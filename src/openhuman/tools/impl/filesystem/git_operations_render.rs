//! Markdown renderers for [`super::git_operations`] results.
//!
//! Split out of `git_operations.rs` for the Rust layout gate. Pure formatting:
//! these take already-parsed JSON and produce the human-readable summary, with
//! no git invocation of their own.

pub(super) fn render_status_markdown(
    result: &serde_json::Map<String, serde_json::Value>,
) -> String {
    let mut out = String::new();
    if let Some(branch) = result.get("branch").and_then(|v| v.as_str()) {
        out.push_str(&format!("**branch**: `{branch}`\n"));
    }
    let clean = result
        .get("clean")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if clean {
        out.push_str("_Working tree clean._\n");
        return out;
    }
    let push_section = |out: &mut String, label: &str, items: Option<&Vec<serde_json::Value>>| {
        if let Some(items) = items {
            if !items.is_empty() {
                out.push_str(&format!("\n**{label}** ({})\n", items.len()));
                for it in items {
                    if let (Some(p), Some(s)) = (
                        it.get("path").and_then(|v| v.as_str()),
                        it.get("status").and_then(|v| v.as_str()),
                    ) {
                        out.push_str(&format!("- `{s}` {p}\n"));
                    }
                }
            }
        }
    };
    push_section(
        &mut out,
        "staged",
        result.get("staged").and_then(|v| v.as_array()),
    );
    push_section(
        &mut out,
        "unstaged",
        result.get("unstaged").and_then(|v| v.as_array()),
    );
    if let Some(items) = result.get("untracked").and_then(|v| v.as_array()) {
        if !items.is_empty() {
            out.push_str(&format!("\n**untracked** ({})\n", items.len()));
            for it in items {
                if let Some(p) = it.as_str() {
                    out.push_str(&format!("- {p}\n"));
                }
            }
        }
    }
    out
}

pub(super) fn render_log_markdown(commits: &[serde_json::Value]) -> String {
    if commits.is_empty() {
        return "_No commits._".to_string();
    }
    let mut out = format!("# Commits ({})\n", commits.len());
    for c in commits {
        let hash = c.get("hash").and_then(|v| v.as_str()).unwrap_or("");
        let short = hash.get(..hash.len().min(8)).unwrap_or(hash);
        let author = c.get("author").and_then(|v| v.as_str()).unwrap_or("");
        let date = c.get("date").and_then(|v| v.as_str()).unwrap_or("");
        let msg = c.get("message").and_then(|v| v.as_str()).unwrap_or("");
        out.push_str(&format!("- `{short}` {msg} _(by {author}, {date})_\n"));
    }
    out
}

pub(super) fn render_branch_markdown(current: &str, branches: &[serde_json::Value]) -> String {
    let mut out = format!("**current**: `{current}`\n\n## Branches\n");
    for b in branches {
        let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let cur = b.get("current").and_then(|v| v.as_bool()).unwrap_or(false);
        if cur {
            out.push_str(&format!("- **{name}** ← current\n"));
        } else {
            out.push_str(&format!("- {name}\n"));
        }
    }
    out
}
