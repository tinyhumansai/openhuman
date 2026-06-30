//! Source-code compressor — keep signatures, collapse bodies.
//!
//! Inspired by Headroom's `CodeCompressor`. The goal is to keep the structural
//! skeleton an agent needs to navigate a file — imports, type/function/class
//! signatures, top-level constants — while collapsing the deep bodies that
//! dominate byte count.
//!
//! This module currently ships the language-agnostic **brace-depth heuristic**:
//! lines at brace nesting depth 0–1 are kept; deeper bodies collapse to a
//! `{ … N lines … }` placeholder. Lines carrying error/TODO markers are always
//! kept. A higher-fidelity tree-sitter path (Rust/TS/Python) is layered on in a
//! follow-up slice and selected by language; until then every language uses the
//! heuristic. The router offloads the original to CCR for exact recovery.

use async_trait::async_trait;
use std::fmt::Write as _;

use super::Compressor;
use crate::openhuman::tokenjuice::types::{
    CompressInput, CompressOptions, CompressOutput, CompressorKind,
};

/// Bodies with more than this many collapsed lines get a placeholder; shorter
/// ones are kept verbatim (collapsing tiny bodies isn't worth the marker).
pub const MIN_BODY_LINES_TO_COLLAPSE: usize = 4;

/// Markers that force a line to be kept even inside a deep body.
const KEEP_MARKERS: &[&str] = &["TODO", "FIXME", "XXX", "error", "panic", "unsafe"];

pub struct CodeCompressor;

#[async_trait]
impl Compressor for CodeCompressor {
    fn kind(&self) -> CompressorKind {
        CompressorKind::Code
    }

    async fn compress(
        &self,
        input: &CompressInput<'_>,
        _opts: &CompressOptions,
    ) -> Option<CompressOutput> {
        // Prefer the AST path when a grammar matches the file's language; fall
        // back to the language-agnostic heuristic otherwise (or when the AST
        // path doesn't shrink the content).
        #[cfg(feature = "tokenjuice-treesitter")]
        if let Some(ext) = input.hint.extension.as_deref() {
            if let Some(out) = treesitter::compress(input.content, ext) {
                return Some(out);
            }
        }
        compress_heuristic(input.content)
    }
}

/// Language-agnostic brace-depth compressor. Keeps lines at depth ≤ 1, collapses
/// deeper runs. Returns `None` if it wouldn't shrink the content.
pub fn compress_heuristic(content: &str) -> Option<CompressOutput> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() < 12 {
        return None;
    }

    let mut out = String::with_capacity(content.len() / 2 + 64);
    let mut depth: i32 = 0;
    let mut collapsed: Vec<&str> = Vec::new();

    let flush = |out: &mut String, collapsed: &mut Vec<&str>| {
        if collapsed.is_empty() {
            return;
        }
        if collapsed.len() >= MIN_BODY_LINES_TO_COLLAPSE {
            let _ = writeln!(out, "    {{ … {} line(s) … }}", collapsed.len());
        } else {
            for l in collapsed.iter() {
                let _ = writeln!(out, "{l}");
            }
        }
        collapsed.clear();
    };

    for line in &lines {
        let (opens, closes) = brace_delta(line);
        let start_depth = depth;
        // A line that carries signal is always kept, regardless of depth.
        let force_keep = KEEP_MARKERS.iter().any(|m| line.contains(m));

        // Keep top-level lines (depth 0) — imports, signatures, the line that
        // opens a block — and collapse the block body (depth ≥ 1). Short bodies
        // (e.g. small struct field lists) stay verbatim via the flush threshold,
        // so struct/enum fields survive while long function bodies collapse.
        if start_depth == 0 || force_keep {
            flush(&mut out, &mut collapsed);
            let _ = writeln!(out, "{line}");
        } else {
            collapsed.push(line);
        }
        depth += opens - closes;
        if depth < 0 {
            depth = 0;
        }
    }
    flush(&mut out, &mut collapsed);

    let out = out.trim_end().to_string();
    if out.len() >= content.len() {
        return None;
    }
    log::debug!(
        "[tokenjuice][code] heuristic {} -> {} bytes ({} lines)",
        content.len(),
        out.len(),
        lines.len()
    );
    Some(CompressOutput::lossy(out, CompressorKind::Code))
}

/// Count `{`/`}` (and `(`/`)`) on a line, ignoring those inside string/char
/// literals and line comments — a cheap approximation good enough for the
/// depth heuristic.
fn brace_delta(line: &str) -> (i32, i32) {
    let mut opens = 0i32;
    let mut closes = 0i32;
    let mut in_str: Option<char> = None;
    let mut prev = '\0';
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match in_str {
            Some(q) => {
                if c == q && prev != '\\' {
                    in_str = None;
                }
            }
            None => match c {
                '"' | '\'' | '`' => in_str = Some(c),
                '/' if chars.peek() == Some(&'/') => break, // line comment
                '#' => break,                               // python/shell comment
                '{' => opens += 1,
                '}' => closes += 1,
                _ => {}
            },
        }
        prev = c;
    }
    (opens, closes)
}

/// AST-aware code compression via tree-sitter (Rust/TS/JS/Python). Keeps full
/// source but replaces function/method bodies longer than a threshold with a
/// `{ … N lines … }` (or `...` for Python) placeholder, preserving signatures,
/// imports, type declarations and struct/enum fields exactly.
#[cfg(feature = "tokenjuice-treesitter")]
mod treesitter {
    use super::{CompressOutput, CompressorKind, MIN_BODY_LINES_TO_COLLAPSE};
    use tree_sitter::{Node, Parser};

    /// Pick the grammar for a file extension. Returns the language plus whether
    /// it is brace-delimited (vs. Python's indentation suite).
    fn language_for(ext: &str) -> Option<(tree_sitter::Language, bool)> {
        let ext = ext.to_ascii_lowercase();
        match ext.as_str() {
            "rs" => Some((tree_sitter_rust::LANGUAGE.into(), true)),
            "ts" | "mts" | "cts" => {
                Some((tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), true))
            }
            "tsx" => Some((tree_sitter_typescript::LANGUAGE_TSX.into(), true)),
            "js" | "jsx" | "mjs" | "cjs" => {
                // The TypeScript grammar is a superset that parses JS too.
                Some((tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(), true))
            }
            "py" | "pyi" => Some((tree_sitter_python::LANGUAGE.into(), false)),
            _ => None,
        }
    }

    /// Node kinds whose `body` field is a collapsible function/method body.
    const BODY_PARENTS: &[&str] = &[
        "function_item",
        "function_declaration",
        "function_definition",
        "method_definition",
        "function",
        "arrow_function",
        "generator_function_declaration",
    ];

    pub fn compress(content: &str, ext: &str) -> Option<CompressOutput> {
        let (language, braced) = language_for(ext)?;
        let mut parser = Parser::new();
        parser.set_language(&language).ok()?;
        let tree = parser.parse(content, None)?;
        let src = content.as_bytes();

        // Collect outermost collapsible body byte-ranges.
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        collect_bodies(tree.root_node(), src, &mut ranges);
        // Sort and drop nested ranges (keep outermost only).
        ranges.sort_by_key(|r| r.0);
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for r in ranges {
            if let Some(last) = merged.last() {
                if r.0 < last.1 {
                    continue; // nested inside a body we're already collapsing
                }
            }
            merged.push(r);
        }
        if merged.is_empty() {
            return None;
        }

        let mut out = String::with_capacity(content.len());
        let mut cursor = 0usize;
        for (start, end) in merged {
            if start < cursor {
                continue;
            }
            out.push_str(&content[cursor..start]);
            let body = &content[start..end];
            let n_lines = body.lines().count();
            if n_lines < MIN_BODY_LINES_TO_COLLAPSE {
                out.push_str(body);
            } else if braced {
                out.push_str(&format!("{{ … {n_lines} line(s) … }}"));
            } else {
                // Python suite — keep an indented ellipsis so it still reads.
                out.push_str(&format!("...  # {n_lines} line(s) collapsed"));
            }
            cursor = end;
        }
        out.push_str(&content[cursor..]);

        let out = out.trim_end().to_string();
        if out.len() >= content.len() {
            return None;
        }
        log::debug!(
            "[tokenjuice][code] tree-sitter ext={} {} -> {} bytes",
            ext,
            content.len(),
            out.len()
        );
        Some(CompressOutput::lossy(out, CompressorKind::Code))
    }

    /// Recursively collect the byte-ranges of function/method bodies.
    fn collect_bodies(node: Node, src: &[u8], out: &mut Vec<(usize, usize)>) {
        if BODY_PARENTS.contains(&node.kind()) {
            if let Some(body) = node.child_by_field_name("body") {
                out.push((body.start_byte(), body.end_byte()));
                // Don't descend into a collapsed body.
                let _ = src;
                return;
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_bodies(child, src, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_signatures_collapses_bodies() {
        let mut src = String::from("use std::collections::HashMap;\n\n");
        src.push_str("pub fn process(items: &[i32]) -> i32 {\n");
        for i in 0..30 {
            src.push_str(&format!(
                "        let tmp_{i} = items.iter().sum::<i32>() + {i};\n"
            ));
        }
        src.push_str("        tmp_0\n}\n\n");
        src.push_str("struct Config {\n    name: String,\n    size: usize,\n}\n");
        let out = compress_heuristic(&src).expect("compresses");
        assert!(out.lossy);
        assert!(
            out.text.contains("pub fn process"),
            "signature kept:\n{}",
            out.text
        );
        assert!(out.text.contains("struct Config"));
        assert!(
            out.text.contains("line(s) …"),
            "body collapsed:\n{}",
            out.text
        );
        assert!(
            !out.text.contains("tmp_15"),
            "deep body should be collapsed"
        );
        assert!(out.text.len() < src.len());
    }

    #[test]
    fn short_file_passes_through() {
        let src = "fn a() {}\nfn b() {}\n";
        assert!(compress_heuristic(src).is_none());
    }

    #[cfg(feature = "tokenjuice-treesitter")]
    #[test]
    fn treesitter_collapses_rust_body_keeps_struct() {
        let mut src = String::from("use std::collections::HashMap;\n\n");
        src.push_str("pub fn process(items: &[i32]) -> i32 {\n");
        for i in 0..30 {
            src.push_str(&format!(
                "    let tmp_{i} = items.iter().sum::<i32>() + {i};\n"
            ));
        }
        src.push_str("    tmp_0\n}\n\n");
        src.push_str("pub struct Config {\n    pub name: String,\n    pub size: usize,\n}\n");
        let out = treesitter::compress(&src, "rs").expect("compresses");
        assert!(
            out.text.contains("pub fn process(items: &[i32]) -> i32"),
            "{}",
            out.text
        );
        // Struct fields preserved exactly (not a function body).
        assert!(out.text.contains("pub name: String"), "{}", out.text);
        assert!(out.text.contains("pub size: usize"));
        // Function body collapsed.
        assert!(out.text.contains("line(s) …"), "{}", out.text);
        assert!(!out.text.contains("tmp_15"));
        assert!(out.text.len() < src.len());
    }

    #[cfg(feature = "tokenjuice-treesitter")]
    #[test]
    fn treesitter_collapses_python_body() {
        let mut src = String::from("import os\n\ndef handler(event):\n");
        for i in 0..30 {
            src.push_str(&format!("    x_{i} = compute(event, {i})\n"));
        }
        src.push_str("    return x_0\n");
        let out = treesitter::compress(&src, "py").expect("compresses");
        assert!(out.text.contains("def handler(event):"), "{}", out.text);
        assert!(out.text.contains("collapsed"), "{}", out.text);
        assert!(!out.text.contains("x_15"));
    }

    #[test]
    fn keeps_marker_lines_in_body() {
        let mut src = String::from("fn f() {\n");
        for i in 0..20 {
            if i == 10 {
                src.push_str("        // TODO: handle the edge case here\n");
            } else {
                src.push_str(&format!("        do_thing({i});\n"));
            }
        }
        src.push_str("}\n");
        let out = compress_heuristic(&src).expect("compresses");
        assert!(out.text.contains("TODO"), "marker line kept:\n{}", out.text);
    }
}
