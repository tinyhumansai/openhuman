//! Conservative, derived file-dependency projection.
//!
//! Inspired by Graphify / CodeGraph, but not a new persistent graph store.
//! Edges express observed syntax and exact catalog matching; absent, duplicated
//! or unsupported resolutions NEVER become confirmed dependencies.
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{json, Value};

/// Original text is only retained for the duration of a bounded graph call.
#[derive(Clone)]
pub(super) struct SourceDocument {
    pub path: String,
    pub source_hash: String,
    pub content: String,
}

fn pattern(cache: &'static OnceLock<Regex>, expression: &'static str) -> &'static Regex {
    cache.get_or_init(|| Regex::new(expression).expect("literal regex is valid"))
}

#[derive(Clone, Copy)]
enum SourceSyntax {
    Rust,
    JavaScript,
    Python,
}

#[derive(Default)]
struct LexicalState {
    block_depth: usize,
    js_template: bool,
    python_triple: Option<u8>,
    rust_raw_hashes: Option<usize>,
    rust_multiline_quote: Option<u8>,
}

/// Conservative lexical mask for candidate statement locations, not an AST.
/// Quoted calls and multiline comments are not dependency statements.
fn code_positions(line: &str, syntax: SourceSyntax, state: &mut LexicalState) -> Vec<bool> {
    let bytes = line.as_bytes();
    let mut visible = vec![false; bytes.len()];
    let mut quote = if matches!(syntax, SourceSyntax::Rust) {
        state.rust_multiline_quote
    } else {
        None
    };
    let mut index = 0;
    while index < bytes.len() {
        let current = bytes[index];
        let next = bytes.get(index + 1).copied();
        // Rust raw strings can contain arbitrary module-looking declarations,
        // braces and comment delimiters across many lines. Keep the exact
        // number of raw-string hashes until a matching closing delimiter.
        if let Some(hashes) = state.rust_raw_hashes {
            if current == b'"'
                && bytes
                    .get(index + 1..index + 1 + hashes)
                    .is_some_and(|tail| tail.iter().all(|byte| *byte == b'#'))
            {
                state.rust_raw_hashes = None;
                index += 1 + hashes;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(triple) = state.python_triple {
            if bytes.get(index..index + 3) == Some(&[triple, triple, triple][..]) {
                state.python_triple = None;
                index += 3;
            } else {
                index += 1;
            }
            continue;
        }
        if state.block_depth > 0 {
            if current == b'/' && next == Some(b'*') && matches!(syntax, SourceSyntax::Rust) {
                state.block_depth += 1;
                index += 2;
            } else if current == b'*' && next == Some(b'/') {
                state.block_depth -= 1;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if state.js_template {
            if current == b'\\' {
                index += 2;
            } else if current == b'\x60' {
                state.js_template = false;
                index += 1;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if current == b'\\' {
                index += 2;
            } else {
                if current == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        // Parse the opening raw-string delimiter exactly: r"...",
        // r###"..."###, br#"..."#. Only run this in Rust, before
        // treating '#', '/', quotes and braces as ordinary code.
        if matches!(syntax, SourceSyntax::Rust) {
            let prefix = if current == b'r' {
                Some(1)
            } else if current == b'b' && next == Some(b'r') {
                Some(2)
            } else {
                None
            };
            if let Some(prefix_len) = prefix {
                let mut cursor = index + prefix_len;
                while bytes.get(cursor) == Some(&b'#') {
                    cursor += 1;
                }
                if bytes.get(cursor) == Some(&b'"') {
                    state.rust_raw_hashes = Some(cursor - index - prefix_len);
                    index = cursor + 1;
                    continue;
                }
            }
        }
        if matches!(syntax, SourceSyntax::Python)
            && matches!(current, b'\'' | b'"')
            && bytes.get(index..index + 3) == Some(&[current, current, current][..])
        {
            state.python_triple = Some(current);
            index += 3;
            continue;
        }
        if matches!(syntax, SourceSyntax::Python) && current == b'#' {
            break;
        }
        if !matches!(syntax, SourceSyntax::Python) && current == b'/' && next == Some(b'/') {
            break;
        }
        if !matches!(syntax, SourceSyntax::Python) && current == b'/' && next == Some(b'*') {
            state.block_depth += 1;
            index += 2;
            continue;
        }
        if matches!(syntax, SourceSyntax::JavaScript) && current == b'\x60' {
            state.js_template = true;
            index += 1;
            continue;
        }
        if matches!(current, b'\'' | b'"') {
            quote = Some(current);
            index += 1;
            continue;
        }
        visible[index] = true;
        index += 1;
    }
    if matches!(syntax, SourceSyntax::Rust) {
        // Ordinary Rust string literals can also span source lines. Do not
        // reset an unclosed double quote at every newline.
        state.rust_multiline_quote = quote.filter(|delimiter| *delimiter == b'"');
    }
    visible
}

fn is_code(captures: &regex::Captures<'_>, group: usize, mask: &[bool]) -> bool {
    captures
        .get(group)
        .and_then(|matched| mask.get(matched.start()))
        .copied()
        .unwrap_or(false)
}

fn rust_mod_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    pattern(
        &PATTERN,
        r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z_0-9]*)\s*;",
    )
}
fn js_from_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    pattern(
        &PATTERN,
        r#"^\s*(?:import|export)\b.*?\b(from)\s*['"](\.[^'"]+)['"]"#,
    )
}
fn js_side_effect_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    pattern(&PATTERN, r#"^\s*import\s*['"](\.[^'"]+)['"]"#)
}
fn js_require_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    pattern(&PATTERN, r#"\brequire\s*\(\s*['"](\.[^'"]+)['"]\s*\)"#)
}

fn js_dynamic_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    pattern(&PATTERN, r#"\bimport\s*\(\s*['"](\.[^'"]+)['"]\s*\)"#)
}
fn python_from_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    pattern(&PATTERN, r"^\s*from\s+(\.+[A-Za-z_0-9.]*)\s+import\s+(.+)$")
}
fn python_relative_target(source: &str, spec: &str) -> Option<String> {
    let dots = spec.bytes().take_while(|byte| *byte == b'.').count();
    if dots == 0 {
        return None;
    }
    let module = spec.get(dots..)?;
    let mut parts: Vec<&str> = source.split('/').collect();
    parts.pop();
    for _ in 1..dots {
        parts.pop()?;
    }
    if parts.is_empty() {
        return None;
    }
    if !module.is_empty() {
        for piece in module.split('.') {
            let valid = !piece.is_empty()
                && piece
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
            if !valid {
                return None;
            }
            parts.push(piece);
        }
    }
    Some(parts.join("/"))
}

/// Normalize an import specifier, not a filesystem path; this performs no I/O.
fn relative_target(source: &str, spec: &str) -> Option<String> {
    if !spec.starts_with('.') || spec.contains('\0') || spec.contains('\\') {
        return None;
    }
    let mut parts: Vec<&str> = source.split('/').collect();
    parts.pop();
    for segment in spec.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

fn candidate_paths(source: &str, spec: &str, kind: &str) -> Vec<String> {
    if kind == "module" {
        // foo.rs nested-module directory convention differs from lib/main/mod.
        let mut parent = source
            .rsplit_once('/')
            .map_or("", |(dir, _)| dir)
            .to_string();
        let name = Path::new(source)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if !matches!(name, "lib" | "main" | "mod") {
            if !parent.is_empty() {
                parent.push('/');
            }
            parent.push_str(name);
        }
        let prefix = if parent.is_empty() {
            String::new()
        } else {
            format!("{parent}/")
        };
        return vec![
            format!("{prefix}{spec}.rs"),
            format!("{prefix}{spec}/mod.rs"),
        ];
    }
    if kind == "python_import" {
        let Some(stem) = python_relative_target(source, spec) else {
            return Vec::new();
        };
        return vec![format!("{stem}.py"), format!("{stem}/__init__.py")];
    }
    let Some(stem) = relative_target(source, spec) else {
        return Vec::new();
    };
    if matches!(
        Path::new(&stem).extension().and_then(|ext| ext.to_str()),
        Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts")
    ) {
        vec![stem]
    } else {
        // A literal resource path may name an observed JSON, CSS or other
        // text asset without a script extension. Include the exact name,
        // then script suffix candidates. Unknown dotted stems such as
        // user.service remain conservative and can be ambiguous.
        std::iter::once(stem.clone())
            .chain(
                ["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"]
                    .iter()
                    .map(|ext| format!("{stem}.{ext}")),
            )
            .chain(
                ["ts", "tsx", "js", "jsx"]
                    .iter()
                    .map(|ext| format!("{stem}/index.{ext}")),
            )
            .collect()
    }
}

fn resolve(
    present: &BTreeMap<&str, &str>,
    candidates: Vec<String>,
) -> (&'static str, Option<String>, usize) {
    let matches: Vec<String> = candidates
        .into_iter()
        .filter(|name| present.contains_key(name.as_str()))
        .collect();
    match matches.len() {
        0 => ("unresolved", None, 0),
        1 => ("resolved", matches.into_iter().next(), 1),
        n => ("ambiguous", None, n),
    }
}

/// A purely derived graph; no filesystem access, LLM call, or persistence.
/// Impact is a bounded lower bound based only on confirmed observed edges.
pub(super) fn build_graph(docs: &[SourceDocument], max_edges: usize, focus: Option<&str>) -> Value {
    let mut ordered: Vec<&SourceDocument> = docs.iter().collect();
    ordered.sort_by(|a, b| a.path.cmp(&b.path));
    let present: BTreeMap<&str, &str> = ordered
        .iter()
        .map(|doc| (doc.path.as_str(), doc.source_hash.as_str()))
        .collect();
    let nodes: Vec<Value> = ordered
        .iter()
        .map(|doc| json!({"path":doc.path,"source_hash":doc.source_hash}))
        .collect();

    let mut edges = Vec::<Value>::new();
    let mut reverse = BTreeMap::<String, BTreeSet<String>>::new();
    let mut complete = true;
    let max_edges = max_edges.min(64);
    'documents: for doc in ordered {
        let ext = Path::new(&doc.path).extension().and_then(|e| e.to_str());
        let js = matches!(
            ext,
            Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts")
        );
        let rust = ext == Some("rs");
        let python = ext == Some("py");
        if !js && !rust && !python {
            continue;
        }
        let mut pending_custom_path = false;
        let mut rust_nesting = 0usize;
        let mut lexical = LexicalState::default();
        let syntax = if rust {
            SourceSyntax::Rust
        } else if js {
            SourceSyntax::JavaScript
        } else {
            SourceSyntax::Python
        };
        for (index, line) in doc.content.lines().enumerate() {
            let code_mask = code_positions(line, syntax, &mut lexical);
            let was_nested = rust && rust_nesting > 0;
            if rust {
                // Ignore nested inline modules instead of resolving their
                // relative paths using the containing file's root directory.
                // This graph intentionally does not parse Rust module scopes.
                for (byte, code) in line.bytes().zip(code_mask.iter().copied()) {
                    if !code {
                        continue;
                    }
                    if byte == b'{' {
                        rust_nesting += 1;
                    } else if byte == b'}' {
                        rust_nesting = rust_nesting.saturating_sub(1);
                    }
                }
            }
            let imports: Vec<(&str, String)> = if rust {
                let trimmed = line.trim_start();
                if trimmed.starts_with("#[path")
                    && line
                        .find("#[path")
                        .is_some_and(|byte| code_mask.get(byte).copied().unwrap_or(false))
                {
                    pending_custom_path = true;
                    continue;
                }
                if pending_custom_path {
                    let no_code = !line
                        .bytes()
                        .zip(code_mask.iter().copied())
                        .any(|(byte, visible)| visible && !byte.is_ascii_whitespace());
                    if trimmed.starts_with("#[") || no_code {
                        continue;
                    }
                    pending_custom_path = false;
                    if rust_mod_pattern().is_match(line) {
                        // Its actual target is controlled by #[path], not by
                        // filename conventions. Exclude rather than guess.
                        continue;
                    }
                }
                if was_nested {
                    continue;
                }
                rust_mod_pattern()
                    .captures(line)
                    .filter(|captures| is_code(captures, 1, &code_mask))
                    .map(|captures| ("module", captures[1].to_string()))
                    .into_iter()
                    .collect()
            } else if js {
                // Conservative: ignore explicit comment-only lines.
                if line.trim_start().starts_with("//") {
                    continue;
                }
                let mut found = Vec::new();
                // The keyword at the start and the from-clause itself must
                // both be visible code. Without the second check, an ordinary
                // "export { x }; // from './fake'" creates a phantom edge.
                if let Some(captures) = js_from_pattern().captures(line).filter(|captures| {
                    is_code(captures, 0, &code_mask) && is_code(captures, 1, &code_mask)
                }) {
                    found.push(("import", captures[2].to_string()));
                }
                if let Some(captures) = js_side_effect_pattern()
                    .captures(line)
                    .or_else(|| js_require_pattern().captures(line))
                    .filter(|captures| is_code(captures, 0, &code_mask))
                {
                    found.push(("import", captures[1].to_string()));
                }
                if let Some(captures) = js_dynamic_pattern()
                    .captures(line)
                    .filter(|captures| is_code(captures, 0, &code_mask))
                {
                    found.push(("dynamic_import", captures[1].to_string()));
                }
                found
            } else {
                let mut found = Vec::new();
                if let Some(captures) = python_from_pattern()
                    .captures(line)
                    .filter(|captures| is_code(captures, 1, &code_mask))
                {
                    let module = &captures[1];
                    if module.bytes().all(|byte| byte == b'.') {
                        for name in captures[2].split(',') {
                            let name = name.split_whitespace().next().unwrap_or("");
                            let valid = name
                                .bytes()
                                .next()
                                .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
                                && name
                                    .bytes()
                                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
                            if valid {
                                found.push(("python_import", format!("{module}{name}")));
                            }
                        }
                    } else {
                        found.push(("python_import", module.to_string()));
                    }
                }
                found
            };
            for (kind, spec) in imports {
                let (status, target, count) =
                    resolve(&present, candidate_paths(&doc.path, &spec, kind));
                if edges.len() >= max_edges {
                    complete = false;
                    break 'documents;
                }
                if let Some(to) = target.as_ref() {
                    reverse
                        .entry(to.clone())
                        .or_default()
                        .insert(doc.path.clone());
                }
                edges.push(json!({
                    "from":doc.path,
                    "to":target,
                    "kind":kind,
                    "status":status,
                    "candidate_count":count,
                    "line":index+1,
                    "specifier":spec.chars().take(120).collect::<String>(),
                    "evidence_hash":doc.source_hash,
                    "confidence":"syntactic_only"
                }));
            }
        }
    }

    let indexed_focus = focus.is_some_and(|path| present.contains_key(path));
    let mut impacted = BTreeSet::new();
    if let Some(focus_path) = focus.filter(|_| indexed_focus) {
        let mut pending = VecDeque::from([focus_path.to_owned()]);
        while let Some(target) = pending.pop_front() {
            if let Some(sources) = reverse.get(&target) {
                for parent in sources {
                    if parent != focus_path && impacted.insert(parent.clone()) {
                        pending.push_back(parent.clone());
                    }
                }
            }
        }
    }

    let mut graph = json!({
        "operation":"graph",
        "nodes":nodes,
        "edges":edges,
        "edges_complete":complete,
        "impact":impacted.into_iter().collect::<Vec<_>>(),
        "focus_indexed":indexed_focus,
        "impact_exhaustive":false,
        "semantic_graph":false,
        "snapshot_atomic":false,
        "supported_syntax":["rust_mod", "javascript_typescript_static_relative_import", "commonjs_relative_require", "javascript_dynamic_literal_import", "python_relative_from_import"],
        "resolution_rule":"exact unique local candidate only"
    });
    // Tool output may be threaded directly into an LLM context. Preserve a
    // parseable JSON envelope instead of relying on downstream character
    // truncation, which would corrupt it and drop completeness metadata.
    const MAX_GRAPH_JSON_BYTES: usize = 26_000;
    while graph.to_string().len() > MAX_GRAPH_JSON_BYTES {
        let Some(edges) = graph["edges"].as_array_mut() else {
            break;
        };
        if edges.pop().is_none() {
            break;
        }
        graph["edges_complete"] = json!(false);
        graph["edge_truncated_reason"] = json!("max_graph_result_bytes");
        // Do not leave dependency impact claims that rely on relationships
        // deliberately omitted from the bounded response.
        graph["impact"] = json!([]);
        graph["impact_omitted_reason"] = json!("bounded_edges");
    }
    graph
}

#[cfg(test)]
#[path = "source_graph_tests.rs"]
mod tests;
