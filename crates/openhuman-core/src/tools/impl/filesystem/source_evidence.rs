//! Read-only, source-hash anchored workspace evidence.
//!
//! Adapted from the evidence/scope contracts of Jev and Conduit. This is not
//! a durable index: each call checks the host SecurityPolicy and observed bytes.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolRunContext};
use tokio::io::AsyncReadExt;

use super::gate::security_scoped_to_root;
use super::source_graph::{build_graph, SourceDocument};
use super::source_locate::rank_sources;
use crate::security::SecurityPolicy;

#[path = "source_evidence_args.rs"]
mod args;
use args::validate_source_args;

const MAX_FILES: usize = 32;
const MAX_FILE_BYTES: u64 = 256 * 1024;
const MAX_TOTAL_BYTES: usize = 2 * 1024 * 1024;
const MAX_MATCHES: usize = 3;
const MAX_LINE_CHARS: usize = 160;
const MAX_DEPTH: usize = 12;
const MAX_VISITED_ENTRIES: usize = 8192;
const MAX_VERIFY_LINES: usize = 80;
const MAX_JSON_RESPONSE_BYTES: usize = 26_000;
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    ".git-crypt",
    "node_modules",
    "target",
    ".next",
    "dist",
    "build",
    ".venv",
    "__pycache__",
    ".terraform",
    ".ssh",
    ".aws",
    ".azure",
    ".kube",
    ".gnupg",
    ".docker",
    ".pki",
    ".config",
    ".openhuman",
    "secrets",
];
const SKIP_FILES: &[&str] = &[
    ".env",
    ".env.local",
    ".env.production",
    ".env.development",
    ".envrc",
    ".npmrc",
    ".pypirc",
    ".netrc",
    ".git-credentials",
    "id_rsa",
    "id_ed25519",
    "credentials.json",
    "secrets.json",
    "token.json",
    "auth.json",
    "service-account.json",
];

pub struct SourceEvidenceTool {
    security: Arc<SecurityPolicy>,
}
impl SourceEvidenceTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    async fn run(
        &self,
        args: Value,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        validate_source_args(&args)?;
        let scoped = match context.and_then(|ctx| ctx.workspace()) {
            Some(ws) => security_scoped_to_root(&self.security, &ws.root),
            None => (*self.security).clone(),
        };
        let op = args
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or("scan");
        let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        if explicitly_targets_symlink(&scoped, path) {
            anyhow::bail!("source evidence cannot follow explicitly addressed symbolic links");
        }
        let resolved = scoped
            .validate_path(path)
            .await
            .map_err(anyhow::Error::msg)?;
        let root = scoped
            .validate_path(".")
            .await
            .map_err(anyhow::Error::msg)?;
        if !resolved.starts_with(&root) {
            anyhow::bail!("source evidence path is outside the active workspace");
        }
        if excluded_ancestor(&root, &resolved) || ignored(&resolved, false) {
            anyhow::bail!("source evidence path is excluded");
        }
        let result = match op {
            "scan" => scan(&scoped, &root, &resolved, &args, false, false).await?,
            "graph" => scan(&scoped, &root, &resolved, &args, true, false).await?,
            "locate" => scan(&scoped, &root, &resolved, &args, false, true).await?,
            "verify" => verify(&scoped, &root, &resolved, &args).await?,
            _ => anyhow::bail!("operation must be scan, locate, graph, or verify"),
        };
        Ok(ToolResult::success(result.to_string()))
    }
}

#[async_trait]
impl Tool for SourceEvidenceTool {
    fn name(&self) -> &str {
        "source_evidence"
    }
    fn description(&self) -> &str {
        "Opt-in read-only source evidence in the active workspace. Scan with a          bounded literal query; locate candidate source files; graph local imports; verify hashes before          reading line-numbered excerpts. The tool makes no network requests,          but outputs may be included in context sent to the active model."
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "operation":{"type":"string","enum":["scan","locate","graph","verify"]},
            "path":{"type":"string","description":"Relative path in current workspace, default '.'"},
            "query":{"type":"string","maxLength":128,"description":"Optional literal substring for scanning"},
            "max_files":{"type":"integer","minimum":1,"maximum":32},
            "max_entries":{"type":"integer","minimum":1,"maximum":8192,"description":"Global directory traversal budget"},
            "after_path":{"type":"string","maxLength":180,"description":"Return paths lexically after this previous-page path"},
            "max_edges":{"type":"integer","minimum":1,"maximum":64},
            "max_results":{"type":"integer","minimum":1,"maximum":5,"description":"At most five source location candidates"},
            "focus_path":{"type":"string","maxLength":180,"description":"Optional indexed path for reverse impact analysis in graph mode"},
            "expected_sha256":{"type":"string","minLength":64,"maxLength":64},
            "start_line":{"type":"integer","minimum":1},
            "end_line":{"type":"integer","minimum":1},
            "start_column":{"type":"integer","minimum":1,"description":"Optional one-based Unicode-scalar column offset for each returned excerpt"}
        },"additionalProperties":false})
    }
    fn max_result_size_chars(&self) -> Option<usize> {
        Some(30_000)
    }
    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.run(args, None).await
    }
    async fn execute_with_context(
        &self,
        args: Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        self.run(args, context).await
    }
}

/// Respect the same no-symlink rule for direct paths and directory walks.
/// Canonicalizing first would erase the link and let explicit scans bypass it.
fn explicitly_targets_symlink(policy: &SecurityPolicy, path: &str) -> bool {
    let requested = Path::new(path);
    let mut cursor = if requested.is_absolute() {
        PathBuf::new()
    } else {
        policy.action_dir.clone()
    };
    for part in requested.components() {
        cursor.push(part);
        if std::fs::symlink_metadata(&cursor)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return true;
        }
    }
    false
}

/// An explicit file path cannot bypass the same deny rules that the
/// directory walker uses. Apply these rules to all descendants, not only
/// the basename of a selected file.
fn excluded_ancestor(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    relative.components().any(|part| {
        let std::path::Component::Normal(name) = part else {
            return false;
        };
        let Some(name) = name.to_str() else {
            return true;
        };
        let lower = name.to_ascii_lowercase();
        SKIP_DIRS.contains(&lower.as_str()) || lower.starts_with(".cache")
    })
}

fn ignored(path: &Path, directory: bool) -> bool {
    let Some(name) = path.file_name().and_then(|x| x.to_str()) else {
        return true;
    };
    let lower = name.to_ascii_lowercase();
    if directory {
        return SKIP_DIRS.contains(&lower.as_str()) || lower.starts_with(".cache");
    }
    SKIP_FILES.contains(&lower.as_str())
        || lower.starts_with(".env.")
        || lower.ends_with(".pem")
        || lower.ends_with(".key")
        || lower.ends_with(".p12")
        || lower.ends_with(".pfx")
        || lower.ends_with(".sqlite")
        || lower.ends_with(".db")
}
fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn relative(root: &Path, path: &Path) -> String {
    let label = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned();
    // A backslash is a literal, valid filename character on Unix. Replacing
    // it with '/' would alias two distinct files into one provenance path.
    // Only normalize on Windows, where backslash really is a separator.
    #[cfg(windows)]
    {
        label.replace('\\', "/")
    }
    #[cfg(not(windows))]
    {
        label
    }
}
fn bounded_line(s: &str) -> String {
    s.chars().take(MAX_LINE_CHARS).collect()
}

/// Return a source excerpt showing the exact literal hit. A line-number-only
/// reference with an unrelated first-160-column prefix is not useful evidence,
/// especially for minified files. Columns count Unicode scalar values.
fn bounded_scan_match(line: &str, query: &str) -> (String, usize, bool) {
    let byte = line.find(query).unwrap_or(0);
    let hit_column = line[..byte].chars().count();
    let line_chars = line.chars().count();
    let left_context = (MAX_LINE_CHARS.saturating_sub(query.chars().count())) / 3;
    let start = if line_chars > MAX_LINE_CHARS {
        hit_column
            .saturating_sub(left_context)
            .min(line_chars - MAX_LINE_CHARS)
    } else {
        0
    };
    let excerpt = line.chars().skip(start).take(MAX_LINE_CHARS).collect();
    (excerpt, start + 1, line_chars > MAX_LINE_CHARS)
}
fn parse_line_range(args: &Value) -> anyhow::Result<(usize, usize)> {
    let start = args.get("start_line").and_then(Value::as_u64).unwrap_or(1);
    let end = args
        .get("end_line")
        .and_then(Value::as_u64)
        .unwrap_or(start.saturating_add(39));
    if start == 0 || end < start || end - start >= MAX_VERIFY_LINES as u64 {
        anyhow::bail!("invalid line range (at most {MAX_VERIFY_LINES} lines)");
    }
    Ok((usize::try_from(start)?, usize::try_from(end)?))
}
#[cfg(unix)]
fn same_file_identity(left: &std::fs::Metadata, right: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

async fn read_checked(policy: &SecurityPolicy, path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    // Authorization is per file, not inherited from the scanned directory.
    let Ok(actual) = policy.validate_path(&path.to_string_lossy()).await else {
        return Ok(None);
    };
    let Ok(initial) = tokio::fs::symlink_metadata(path).await else {
        return Ok(None);
    };
    if !initial.is_file() || initial.file_type().is_symlink() || initial.len() > MAX_FILE_BYTES {
        return Ok(None);
    }
    // Limit reads at the OS handle, not after unbounded read_to_end; a file
    // can grow between metadata and reading. On Unix do not follow a symlink
    // substituted at the canonical final component after initial validation.
    // Use Tokio's read-only File::options to make the no-write contract
    // explicit; this is not a persisted-storage writer or an allowance.
    #[cfg(unix)]
    let opened = tokio::fs::File::options()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&actual)
        .await;
    #[cfg(not(unix))]
    let opened = tokio::fs::File::open(&actual).await;
    let Ok(file) = opened else {
        return Ok(None);
    };
    let Ok(metadata) = file.metadata().await else {
        return Ok(None);
    };
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return Ok(None);
    }
    // The path can be replaced after validation but before open. Compare
    // the opened descriptor's identity to the initially authorized file,
    // not just the descriptor's size/type. Reject substitutions before
    // returning any source bytes to the model-facing evidence layer.
    #[cfg(unix)]
    if !same_file_identity(&initial, &metadata) {
        return Ok(None);
    }
    let mut bounded = file.take(MAX_FILE_BYTES + 1);
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    if bounded.read_to_end(&mut bytes).await.is_err() {
        return Ok(None);
    }
    if bytes.len() > MAX_FILE_BYTES as usize
        || bytes.contains(&0)
        || std::str::from_utf8(&bytes).is_err()
    {
        return Ok(None);
    }
    // Fail closed if a symlink or parent was retargeted during the read.
    if policy
        .validate_path(&path.to_string_lossy())
        .await
        .ok()
        .as_ref()
        != Some(&actual)
    {
        return Ok(None);
    }
    #[cfg(unix)]
    {
        // A parent can briefly be retargeted and then restored before the
        // canonical-path comparison. Recheck the current file against the
        // same descriptor that supplied the evidence bytes.
        let Ok(current) = tokio::fs::metadata(path).await else {
            return Ok(None);
        };
        if !same_file_identity(&metadata, &current) {
            return Ok(None);
        }
    }
    Ok(Some(bytes))
}
async fn scan(
    policy: &SecurityPolicy,
    root: &Path,
    path: &Path,
    args: &Value,
    graph_mode: bool,
    locate_mode: bool,
) -> anyhow::Result<Value> {
    let query = args.get("query").and_then(Value::as_str).unwrap_or("");
    if query.len() > 128 {
        anyhow::bail!("query too long");
    }
    if locate_mode && query.trim().is_empty() {
        anyhow::bail!("locate requires a nonempty query");
    }
    let max_files = args
        .get("max_files")
        .and_then(Value::as_u64)
        .unwrap_or(MAX_FILES as u64)
        .clamp(1, MAX_FILES as u64) as usize;
    let max_entries = args
        .get("max_entries")
        .and_then(Value::as_u64)
        .unwrap_or(MAX_VISITED_ENTRIES as u64)
        .clamp(1, MAX_VISITED_ENTRIES as u64) as usize;
    let after = args.get("after_path").and_then(Value::as_str).unwrap_or("");
    // A backslash is a separator on Windows but a valid literal filename
    // character on Unix. Cursors use the same canonical representation as
    // relative() so a client can round-trip every reported safe path.
    let invalid_backslash = cfg!(windows) && after.contains('\\');
    if after.len() > 180
        || after.starts_with('/')
        || invalid_backslash
        || after.chars().any(char::is_control)
        || after
            .split('/')
            .any(|part| part == ".." || (part.is_empty() && !after.is_empty()))
    {
        anyhow::bail!("invalid after_path cursor");
    }
    let mut paths = vec![(path.to_path_buf(), 0usize)];
    let mut entries: Vec<PathBuf> = Vec::new();
    let mut truncated: Option<&str> = None;
    // Bounded traversal; never follow directory links. Output is ordered
    // within the observed window but a directory entry-limit is partial.
    let mut visited = 0usize;
    while let Some((next, depth)) = paths.pop() {
        visited += 1;
        if visited > max_entries {
            truncated = Some("max_entries");
            break;
        }
        // JSON paths cannot round-trip arbitrary byte sequences through a
        // lossy UTF-8 formatter. Do not silently omit such files and then
        // report a complete source catalog.
        if next
            .strip_prefix(root)
            .unwrap_or(&next)
            .components()
            .any(|part| part.as_os_str().to_str().is_none())
        {
            truncated.get_or_insert("non_utf8_path");
            continue;
        }
        let metadata = match tokio::fs::symlink_metadata(&next).await {
            Ok(meta) => meta,
            Err(_) => {
                truncated = Some("filesystem_changed");
                continue;
            }
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_file() {
            if !ignored(&next, false)
                && (after.is_empty() || relative(root, &next).as_str() > after)
            {
                entries.push(next);
            }
            // Collect all files within the independent traversal budget
            // before selecting a page. A depth-first walk is not globally
            // sorted: a/child.rs can be visited before a.rs although a.rs
            // precedes it in the cursor order.
        } else if metadata.is_dir() && !ignored(&next, true) {
            if depth >= MAX_DEPTH {
                truncated = Some("max_depth");
                continue;
            }
            if policy.validate_path(&next.to_string_lossy()).await.is_err() {
                continue;
            }
            let mut read = tokio::fs::read_dir(&next).await?;
            let mut children = Vec::new();
            while let Some(child) = read.next_entry().await? {
                children.push(child.path());
                if children.len() > 4096 {
                    truncated = Some("directory_entry_limit");
                    break;
                }
            }
            children.sort();
            for child in children.into_iter().rev() {
                paths.push((child, depth + 1));
            }
        }
    }
    entries.sort_by_cached_key(|path| relative(root, path));
    // Only offer a continuation cursor when every eligible name was seen.
    // Entry/depth/fanout truncation means an unseen earlier name may exist.
    if entries.len() > max_files && truncated.is_none() {
        truncated = Some("max_files");
    }
    let mut files = Vec::new();
    let mut graph_docs = Vec::new();
    let mut total = 0usize;
    let mut generation = Sha256::new();
    let mut source_bytes = 0usize;
    let mut last_scanned: Option<String> = None;
    for path in entries.into_iter().take(max_files) {
        let metadata = match tokio::fs::symlink_metadata(&path).await {
            Ok(value) => value,
            Err(_) => {
                truncated.get_or_insert("filesystem_changed");
                continue;
            }
        };
        // Preserve a resume point for each inspected/skipped file, but never
        // advance past a file refused by the *total* byte ceiling.
        let label = relative(root, &path);
        if metadata.len() > MAX_FILE_BYTES {
            truncated.get_or_insert("oversize_file");
            last_scanned = Some(label);
            continue;
        }
        let Some(bytes) = read_checked(policy, &path).await? else {
            truncated.get_or_insert("unreadable_or_unsupported_file");
            last_scanned = Some(label);
            continue;
        };
        if total.saturating_add(bytes.len()) > MAX_TOTAL_BYTES {
            truncated = Some("max_total_bytes");
            break;
        }
        total += bytes.len();
        last_scanned = Some(label.clone());
        let digest = hex_digest(&bytes);
        if label.len() > 180 {
            truncated.get_or_insert("path_length_limit");
            continue;
        }
        generation.update(label.as_bytes());
        generation.update([0]);
        generation.update(digest.as_bytes());
        generation.update([0]);
        let text = std::str::from_utf8(&bytes)?;
        if graph_mode || locate_mode {
            graph_docs.push(SourceDocument {
                path: label.clone(),
                source_hash: digest.clone(),
                content: text.to_owned(),
            });
        }
        let matches: Vec<Value> = if query.is_empty() || locate_mode || graph_mode {
            Vec::new()
        } else {
            let all: Vec<_> = text
                .lines()
                .enumerate()
                .filter(|(_, line)| line.contains(query))
                .collect();
            if all.len() > MAX_MATCHES {
                truncated.get_or_insert("max_matches_per_file");
            }
            all.into_iter()
                .take(MAX_MATCHES)
                .map(|(i, line)| {
                    let (text, column_start, excerpt_truncated) = bounded_scan_match(line, query);
                    json!({
                        "line": i + 1, "text": text, "column_start": column_start,
                        "excerpt_truncated": excerpt_truncated
                    })
                })
                .collect()
        };
        source_bytes += bytes.len();
        files.push(
            json!({"path":label,"source_hash":digest,"size_bytes":bytes.len(),"matches":matches}),
        );
    }
    let generation = format!("{:x}", generation.finalize());
    let next_after_path = if matches!(truncated, Some("max_files" | "max_total_bytes")) {
        last_scanned
    } else {
        None
    };
    // Lexical page cursors and explicit subdirectory selections only observe
    // part of the workspace: they cannot justify unique repository edges.
    let catalog_complete = truncated.is_none() && after.is_empty() && path == root;
    if locate_mode {
        // Only individually authorized, bounded, UTF-8 source bytes enter
        // this local deterministic ranker. It never calls a remote model or
        // persists another index. Its results are *candidates* to verify.
        let limit = args
            .get("max_results")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(1, 5) as usize;
        let mut located = rank_sources(&graph_docs, query, limit);
        located["generation"] = json!(generation);
        located["scope_path"] = json!(relative(root, path));
        located["scope_is_workspace"] = json!(path == root);
        located["catalog_complete"] = json!(catalog_complete);
        located["ranking_exhaustive"] = json!(false);
        located["after_path"] = json!(after);
        located["next_after_path"] = json!(next_after_path);
        located["scan_complete"] = json!(catalog_complete);
        located["truncated_reason"] = json!(truncated);
        located["observed_bytes"] = json!(source_bytes);
        return Ok(located);
    }
    if graph_mode {
        let max_edges = args
            .get("max_edges")
            .and_then(Value::as_u64)
            .unwrap_or(64)
            .clamp(1, 64) as usize;
        let focus = args.get("focus_path").and_then(Value::as_str);
        if focus.is_some_and(|path| path.len() > 180 || path.chars().any(char::is_control)) {
            anyhow::bail!("invalid focus_path");
        }
        let mut graph = build_graph(&graph_docs, max_edges, focus);
        if !catalog_complete {
            // An observed unique candidate is not unique in a catalog whose
            // unscanned files could contain a second matching implementation.
            // Incomplete evidence must not justify definite edges or impact.
            if let Some(edges) = graph["edges"].as_array_mut() {
                for edge in edges {
                    if !edge["to"].is_null() {
                        // A target appeared among observed files, but the
                        // incomplete catalog cannot prove it is the unique
                        // resolution. Preserve it only as an explicitly
                        // provisional candidate, never a confirmed graph edge.
                        edge["observed_candidate"] = edge["to"].clone();
                    }
                    edge["status"] = json!("catalog_incomplete");
                    edge["to"] = Value::Null;
                }
            }
            graph["impact"] = json!([]);
            graph["impact_omitted_reason"] = json!("incomplete_catalog");
        }
        graph["generation"] = json!(generation);
        graph["scope_path"] = json!(relative(root, path));
        graph["scope_is_workspace"] = json!(path == root);
        graph["after_path"] = json!(after);
        graph["catalog_complete"] = json!(catalog_complete);
        graph["next_after_path"] = json!(next_after_path);
        graph["scan_complete"] = json!(catalog_complete);
        graph["truncated_reason"] = json!(truncated);
        graph["observed_bytes"] = json!(source_bytes);
        if !catalog_complete {
            graph["edges_complete"] = json!(false);
        }
        // build_graph bounds its own output, but adding scope and observed
        // candidates can enlarge the returned graph. Bound the FINAL envelope
        // before TinyTools applies its text-based output clipping.
        while graph.to_string().len() > MAX_JSON_RESPONSE_BYTES {
            let Some(removed) = graph["edges"].as_array_mut().and_then(Vec::pop) else {
                anyhow::bail!("graph node envelope exceeds the bounded output budget");
            };
            let _ = removed;
            graph["edges_complete"] = json!(false);
            graph["edge_truncated_reason"] = json!("max_response_bytes");
            graph["impact"] = json!([]);
            graph["impact_omitted_reason"] = json!("bounded_edges");
        }
        return Ok(graph);
    }
    let mut response = json!({
        "operation":"scan","generation":generation,
        "complete":truncated.is_none(),"catalog_complete":catalog_complete,
        "after_path":after,"truncated_reason":truncated,
        "next_after_path":next_after_path,
        "observed_bytes":source_bytes, "files":files,
        "snapshot_atomic":false
    });
    // Maintain JSON validity: downstream tool-output clipping is text-based.
    while response.to_string().len() > MAX_JSON_RESPONSE_BYTES {
        let Some(matches) = response["files"].as_array_mut().and_then(|files| {
            files.iter_mut().rev().find_map(|file| {
                file["matches"]
                    .as_array_mut()
                    .filter(|matches| !matches.is_empty())
            })
        }) else {
            break;
        };
        matches.pop();
        response["complete"] = json!(false);
        response["truncated_reason"] = json!("max_response_bytes");
    }
    Ok(response)
}
async fn verify(
    policy: &SecurityPolicy,
    root: &Path,
    path: &Path,
    args: &Value,
) -> anyhow::Result<Value> {
    let expected = args
        .get("expected_sha256")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("expected_sha256 is required"))?;
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        anyhow::bail!("expected_sha256 must be a 64-digit hexadecimal SHA-256");
    }
    let (start, end) = parse_line_range(args)?;
    let column = args
        .get("start_column")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    if column == 0 || column > MAX_FILE_BYTES + 1 {
        anyhow::bail!("start_column is outside the source evidence budget");
    }
    let skip_columns = usize::try_from(column - 1)?;
    if ignored(path, false) {
        anyhow::bail!("source file is excluded");
    }
    let bytes = read_checked(policy, path)
        .await?
        .ok_or_else(|| anyhow::anyhow!("source file is not permitted, not UTF-8, or too large"))?;
    let actual = hex_digest(&bytes);
    if !actual.eq_ignore_ascii_case(expected) {
        anyhow::bail!("source changed since evidence was recorded");
    }
    let text = std::str::from_utf8(&bytes)?;
    let lines: Vec<Value> = text
        .lines()
        .enumerate()
        .filter(|(i, _)| i + 1 >= start && *i < end)
        .map(|(i, line)| {
            json!({
                "line": i + 1,
                "column_start": column,
                "line_char_count": line.chars().count(),
                "text": line.chars().skip(skip_columns).take(MAX_LINE_CHARS).collect::<String>()
            })
        })
        .collect();
    let mut result = json!({"operation":"verify","path":relative(root,path),"source_hash":actual,
        "verified":true,"lines":lines,"lines_complete":true,"next_line":Value::Null,
        "snapshot_atomic":false});
    while result.to_string().len() > MAX_JSON_RESPONSE_BYTES {
        let Some(removed) = result["lines"].as_array_mut().and_then(Vec::pop) else {
            anyhow::bail!("source verification result cannot fit the tool output budget");
        };
        result["lines_complete"] = json!(false);
        result["next_line"] = removed["line"].clone();
    }
    Ok(result)
}

#[cfg(test)]
#[path = "source_evidence_tests.rs"]
mod tests;
