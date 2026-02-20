//! RAG pipeline for hardware datasheet retrieval.
//!
//! Supports:
//! - Markdown and text datasheets (always)
//! - PDF ingestion (with `rag-pdf` feature)
//! - Pin/alias tables (e.g. `red_led: 13`) for explicit lookup
//! - Keyword retrieval (default) or semantic search via embeddings (optional)

use crate::alphahuman::memory::chunker;
use std::collections::HashMap;
use std::path::Path;

/// A chunk of datasheet content with board metadata.
#[derive(Debug, Clone)]
pub struct DatasheetChunk {
    /// Board this chunk applies to (e.g. "nucleo-f401re", "rpi-gpio"), or None for generic.
    pub board: Option<String>,
    /// Source file path (for debugging).
    pub source: String,
    /// Chunk content.
    pub content: String,
}

/// Pin alias: human-readable name → pin number (e.g. "red_led" → 13).
pub type PinAliases = HashMap<String, u32>;

/// Parse pin aliases from markdown. Looks for:
/// - `## Pin Aliases` section with `alias: pin` lines
/// - Markdown table `| alias | pin |`
fn parse_pin_aliases(content: &str) -> PinAliases {
    let mut aliases = PinAliases::new();
    let content_lower = content.to_lowercase();

    // Find ## Pin Aliases section
    let section_markers = ["## pin aliases", "## pin alias", "## pins"];
    let mut in_section = false;
    let mut section_start = 0;

    for marker in section_markers {
        if let Some(pos) = content_lower.find(marker) {
            in_section = true;
            section_start = pos + marker.len();
            break;
        }
    }

    if !in_section {
        return aliases;
    }

    let rest = &content[section_start..];
    let section_end = rest
        .find("\n## ")
        .map(|i| section_start + i)
        .unwrap_or(content.len());
    let section = &content[section_start..section_end];

    // Parse "alias: pin" or "alias = pin" lines
    for line in section.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Table row: | red_led | 13 | (skip header | alias | pin | and separator |---|)
        if line.starts_with('|') {
            let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
            if parts.len() >= 3 {
                let alias = parts[1].trim().to_lowercase().replace(' ', "_");
                let pin_str = parts[2].trim();
                // Skip header row and separator (|---|)
                if alias.eq("alias")
                    || alias.eq("pin")
                    || pin_str.eq("pin")
                    || alias.contains("---")
                    || pin_str.contains("---")
                {
                    continue;
                }
                if let Ok(pin) = pin_str.parse::<u32>() {
                    if !alias.is_empty() {
                        aliases.insert(alias, pin);
                    }
                }
            }
            continue;
        }
        // Key: value
        if let Some((k, v)) = line.split_once(':').or_else(|| line.split_once('=')) {
            let alias = k.trim().to_lowercase().replace(' ', "_");
            if let Ok(pin) = v.trim().parse::<u32>() {
                if !alias.is_empty() {
                    aliases.insert(alias, pin);
                }
            }
        }
    }

    aliases
}

fn collect_md_txt_paths(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_md_txt_paths(&path, out);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str());
            if ext == Some("md") || ext == Some("txt") {
                out.push(path);
            }
        }
    }
}

#[cfg(feature = "rag-pdf")]
fn collect_pdf_paths(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_pdf_paths(&path, out);
        } else if path.is_file() {
            if path.extension().and_then(|e| e.to_str()) == Some("pdf") {
                out.push(path);
            }
        }
    }
}

#[cfg(feature = "rag-pdf")]
fn extract_pdf_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    pdf_extract::extract_text_from_mem(&bytes).ok()
}

/// Hardware RAG index — loads and retrieves datasheet chunks.
pub struct HardwareRag {
    chunks: Vec<DatasheetChunk>,
    /// Per-board pin aliases (board -> alias -> pin).
    pin_aliases: HashMap<String, PinAliases>,
}

impl HardwareRag {
    /// Load datasheets from a directory. Expects .md, .txt, and optionally .pdf (with rag-pdf).
    /// Filename (without extension) is used as board tag.
    /// Supports `## Pin Aliases` section for explicit alias→pin mapping.
    pub fn load(workspace_dir: &Path, datasheet_dir: &str) -> anyhow::Result<Self> {
        let base = workspace_dir.join(datasheet_dir);
        if !base.exists() || !base.is_dir() {
            return Ok(Self {
                chunks: Vec::new(),
                pin_aliases: HashMap::new(),
            });
        }

        let mut paths: Vec<std::path::PathBuf> = Vec::new();
        collect_md_txt_paths(&base, &mut paths);
        #[cfg(feature = "rag-pdf")]
        collect_pdf_paths(&base, &mut paths);

        let mut chunks = Vec::new();
        let mut pin_aliases: HashMap<String, PinAliases> = HashMap::new();
        let max_tokens = 512;

        for path in paths {
            let content = if path.extension().and_then(|e| e.to_str()) == Some("pdf") {
                #[cfg(feature = "rag-pdf")]
                {
                    extract_pdf_text(&path).unwrap_or_default()
                }
                #[cfg(not(feature = "rag-pdf"))]
                {
                    String::new()
                }
            } else {
                std::fs::read_to_string(&path).unwrap_or_default()
            };

            if content.trim().is_empty() {
                continue;
            }

            let board = infer_board_from_path(&path, &base);
            let source = path
                .strip_prefix(workspace_dir)
                .unwrap_or(&path)
                .display()
                .to_string();

            // Parse pin aliases from full content
            let aliases = parse_pin_aliases(&content);
            if let Some(ref b) = board {
                if !aliases.is_empty() {
                    pin_aliases.insert(b.clone(), aliases);
                }
            }

            for chunk in chunker::chunk_markdown(&content, max_tokens) {
                chunks.push(DatasheetChunk {
                    board: board.clone(),
                    source: source.clone(),
                    content: chunk.content,
                });
            }
        }

        Ok(Self {
            chunks,
            pin_aliases,
        })
    }

    /// Get pin aliases for a board (e.g. "red_led" -> 13).
    pub fn pin_aliases_for_board(&self, board: &str) -> Option<&PinAliases> {
        self.pin_aliases.get(board)
    }

    /// Build pin-alias context for query. When user says "red led", inject "red_led: 13" for matching boards.
    pub fn pin_alias_context(&self, query: &str, boards: &[String]) -> String {
        let query_lower = query.to_lowercase();
        let query_words: Vec<&str> = query_lower
            .split_whitespace()
            .filter(|w| w.len() > 1)
            .collect();

        let mut lines = Vec::new();
        for board in boards {
            if let Some(aliases) = self.pin_aliases.get(board) {
                for (alias, pin) in aliases {
                    let alias_words: Vec<&str> = alias.split('_').collect();
                    let matches = query_words.iter().any(|qw| alias_words.contains(qw))
                        || query_lower.contains(&alias.replace('_', " "));
                    if matches {
                        lines.push(format!("{board}: {alias} = pin {pin}"));
                    }
                }
            }
        }
        if lines.is_empty() {
            return String::new();
        }
        format!("[Pin aliases for query]\n{}\n\n", lines.join("\n"))
    }

    /// Retrieve chunks relevant to the query and boards.
    /// Uses keyword matching and board filter. Pin-alias context is built separately via `pin_alias_context`.
    pub fn retrieve(&self, query: &str, boards: &[String], limit: usize) -> Vec<&DatasheetChunk> {
        if self.chunks.is_empty() || limit == 0 {
            return Vec::new();
        }

        let query_lower = query.to_lowercase();
        let query_terms: Vec<&str> = query_lower
            .split_whitespace()
            .filter(|w| w.len() > 2)
            .collect();

        let mut scored: Vec<(&DatasheetChunk, f32)> = Vec::new();
        for chunk in &self.chunks {
            let content_lower = chunk.content.to_lowercase();
            let mut score = 0.0f32;

            for term in &query_terms {
                if content_lower.contains(term) {
                    score += 1.0;
                }
            }

            if score > 0.0 {
                let board_match = chunk.board.as_ref().map_or(false, |b| boards.contains(b));
                if board_match {
                    score += 2.0;
                }
                scored.push((chunk, score));
            }
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored.into_iter().map(|(c, _)| c).collect()
    }

    /// Number of indexed chunks.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// True if no chunks are indexed.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
}

/// Infer board tag from file path. `nucleo-f401re.md` → Some("nucleo-f401re").
fn infer_board_from_path(path: &Path, base: &Path) -> Option<String> {
    let rel = path.strip_prefix(base).ok()?;
    let stem = path.file_stem()?.to_str()?;

    if stem == "generic" || stem.starts_with("generic_") {
        return None;
    }
    if rel.parent().and_then(|p| p.to_str()) == Some("_generic") {
        return None;
    }

    Some(stem.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pin_aliases_key_value() {
        let md = r#"## Pin Aliases
red_led: 13
builtin_led: 13
user_led: 5"#;
        let a = parse_pin_aliases(md);
        assert_eq!(a.get("red_led"), Some(&13));
        assert_eq!(a.get("builtin_led"), Some(&13));
        assert_eq!(a.get("user_led"), Some(&5));
    }

    #[test]
    fn parse_pin_aliases_table() {
        let md = r#"## Pin Aliases
| alias | pin |
|-------|-----|
| red_led | 13 |
| builtin_led | 13 |"#;
        let a = parse_pin_aliases(md);
        assert_eq!(a.get("red_led"), Some(&13));
        assert_eq!(a.get("builtin_led"), Some(&13));
    }

    #[test]
    fn parse_pin_aliases_empty() {
        let a = parse_pin_aliases("No aliases here");
        assert!(a.is_empty());
    }

    #[test]
    fn infer_board_from_path_nucleo() {
        let base = std::path::Path::new("/base");
        let path = std::path::Path::new("/base/nucleo-f401re.md");
        assert_eq!(
            infer_board_from_path(path, base),
            Some("nucleo-f401re".into())
        );
    }

    #[test]
    fn infer_board_generic_none() {
        let base = std::path::Path::new("/base");
        let path = std::path::Path::new("/base/generic.md");
        assert_eq!(infer_board_from_path(path, base), None);
    }

    #[test]
    fn hardware_rag_load_and_retrieve() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("datasheets");
        std::fs::create_dir_all(&base).unwrap();
        let content = r#"# Test Board
## Pin Aliases
red_led: 13
## GPIO
Pin 13: LED
"#;
        std::fs::write(base.join("test-board.md"), content).unwrap();

        let rag = HardwareRag::load(tmp.path(), "datasheets").unwrap();
        assert!(!rag.is_empty());
        let boards = vec!["test-board".to_string()];
        let chunks = rag.retrieve("led", &boards, 5);
        assert!(!chunks.is_empty());
        let ctx = rag.pin_alias_context("red led", &boards);
        assert!(ctx.contains("13"));
    }

    #[test]
    fn hardware_rag_load_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("empty_ds");
        std::fs::create_dir_all(&base).unwrap();
        let rag = HardwareRag::load(tmp.path(), "empty_ds").unwrap();
        assert!(rag.is_empty());
    }
}
