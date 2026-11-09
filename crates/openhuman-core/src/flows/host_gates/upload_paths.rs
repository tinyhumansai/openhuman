use super::graph_json::nodes;
use serde_json::Value;

/// Author-gate for `oh:storage_upload_file`: its literal `path` arg must be
/// workspace-relative. Uploads are confined to the agent workspace by the
/// runtime `resolve_upload_path` (a canonicalized path that escapes `action_dir`
/// is rejected), so an absolute path like `/tmp/report.html` or one climbing out
/// with `..` cannot work — it fails mid-run at the upload step. The prompt tells
/// the builder to use a relative path, but the model reliably ignores that and
/// copies an absolute path from a prior flow's example, so this enforces it in
/// code (a hard, actionable author-gate) rather than trusting the prose.
///
/// Only LITERAL paths are checked: a `=`-expression resolves from upstream data
/// at runtime and is out of scope here (the runtime check still applies). An
/// absent `path` is left to the required-arg gate.
pub(crate) fn validate_upload_paths(graph: &Value) -> Vec<String> {
    const UPLOAD_SLUG: &str = "oh:storage_upload_file";
    let mut errors = Vec::new();
    for node in nodes(graph) {
        if node.kind != "tool_call" {
            continue;
        }
        if node.config.get("slug").and_then(Value::as_str) != Some(UPLOAD_SLUG) {
            continue;
        }
        let Some(raw) = node
            .config
            .get("args")
            .and_then(|a| a.get("path"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let path = raw.trim();
        // Dynamic (resolved at runtime) or absent — not a literal we can check here.
        if path.is_empty() || path.starts_with('=') {
            continue;
        }
        let escapes_via_parent = path.split(['/', '\\']).any(|seg| seg == "..");
        if std::path::Path::new(path).is_absolute() || escapes_via_parent {
            errors.push(format!(
                "Node '{}': `oh:storage_upload_file` path `{path}` must be workspace-relative \
                 (e.g. `report.html`). Uploads are confined to the agent workspace, so an \
                 absolute path (`/tmp/...`, `/Users/...`) or one escaping with `..` is rejected \
                 at run time. Use a relative path, and have the producing node write the file to \
                 that same relative path.",
                node.id
            ));
        }
    }
    errors
}
