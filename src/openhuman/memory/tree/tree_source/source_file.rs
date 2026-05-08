//! Per-source `_source.md` registry mirror.
//!
//! Sits at `<content_root>/raw/<source_slug>/_source.md` next to the
//! per-kind raw subdirs (`emails/`, `chats/`, `documents/`, …). The file
//! is **frontmatter-only** — its YAML head is the registry record for
//! one source, the body is intentionally empty so Obsidian / `.base`
//! files can render it without distractions.
//!
//! Today this is a *mirror* of the `mem_tree_trees` row for the source's
//! tree (kind + scope + last_sealed_at). SQLite remains the source of
//! truth; the file is rewritten whenever the registry creates or
//! refreshes a tree so the on-disk view stays current. The contract is
//! one-way: nothing reads back from this file at runtime.
//!
//! Future direction: as more per-source state moves out of SQLite (the
//! sibling `tree_source/store.rs` rows that are naturally one-row-per
//! source), this file becomes the load-into-memory authority and the
//! SQLite columns get retired. We keep that migration small and explicit
//! by gating it behind callers; this module just owns the on-disk shape.
//!
//! Atomicity: writes go through the same tempfile-+-rename pattern the
//! sibling `content_store::raw` writer uses, so a crash mid-write leaves
//! either the previous file intact or no file at all — never a partial
//! one.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::content_store::raw::raw_source_dir;
use crate::openhuman::memory::tree::tree_source::types::{Tree, TreeKind, TreeStatus};

/// Filename of the per-source registry mirror inside `raw/<source_slug>/`.
pub const SOURCE_FILE_NAME: &str = "_source.md";

/// Resolve the absolute path of `_source.md` for `source_id` under the
/// configured content root.
pub fn source_file_path(config: &Config, source_id: &str) -> PathBuf {
    let root = config.memory_tree_content_root();
    raw_source_dir(&root, source_id).join(SOURCE_FILE_NAME)
}

/// Render the YAML frontmatter for a tree row. Body is empty — this is a
/// metadata-only file. Field order is fixed so re-renders for the same
/// row produce byte-identical output (idempotent rewrites, clean diffs).
fn render(tree: &Tree) -> String {
    let mut out = String::with_capacity(256);
    out.push_str("---\n");
    out.push_str(&format!("tree_id: {}\n", yaml_scalar(&tree.id)));
    out.push_str(&format!("kind: {}\n", tree.kind.as_str()));
    out.push_str(&format!("scope: {}\n", yaml_scalar(&tree.scope)));
    out.push_str(&format!("status: {}\n", tree.status.as_str()));
    out.push_str(&format!("max_level: {}\n", tree.max_level));
    out.push_str(&format!("created_at: {}\n", iso8601(tree.created_at)));
    match tree.last_sealed_at {
        Some(t) => out.push_str(&format!("last_sealed_at: {}\n", iso8601(t))),
        None => out.push_str("last_sealed_at: null\n"),
    }
    match tree.root_id.as_ref() {
        Some(id) => out.push_str(&format!("root_id: {}\n", yaml_scalar(id))),
        None => out.push_str("root_id: null\n"),
    }
    out.push_str("---\n");
    out
}

fn iso8601(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Quote a YAML scalar if it contains characters that would otherwise
/// break the parse (colons, leading whitespace, quote chars). The
/// scalars we emit (tree ids, scopes) are user-derived, so a defensive
/// quote keeps Obsidian's parser from misreading e.g. `gmail:foo` as a
/// nested mapping.
fn yaml_scalar(s: &str) -> String {
    let needs_quote = s.is_empty()
        || s.contains(':')
        || s.contains('#')
        || s.contains('"')
        || s.contains('\'')
        || s.starts_with(|c: char| c.is_whitespace())
        || s.ends_with(|c: char| c.is_whitespace());
    if !needs_quote {
        return s.to_string();
    }
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Write (or rewrite) `_source.md` for `tree`. Idempotent: rewriting
/// with the same tree state produces the same bytes. Creates parent
/// directories as needed so callers don't have to.
pub fn write_source_file(config: &Config, tree: &Tree) -> Result<PathBuf> {
    let path = source_file_path(config, &tree.scope);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("source file path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("create source file dir {}", parent.display()))?;
    let bytes = render(tree);
    write_atomic(&path, bytes.as_bytes())
        .with_context(|| format!("write source file {}", path.display()))?;
    Ok(path)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("path has no parent: {}", path.display()))?;
    let tmp = parent.join(format!(
        ".tmp_source_{}_{}.md",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut f = fs::File::create(&tmp).with_context(|| format!("create tmp {}", tmp.display()))?;
    f.write_all(bytes)
        .with_context(|| format!("write tmp {}", tmp.display()))?;
    f.sync_all()
        .with_context(|| format!("fsync tmp {}", tmp.display()))?;
    drop(f);
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn cfg() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    fn sample_tree(scope: &str) -> Tree {
        Tree {
            id: "source:abc".into(),
            kind: TreeKind::Source,
            scope: scope.into(),
            root_id: None,
            max_level: 0,
            status: TreeStatus::Active,
            created_at: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
            last_sealed_at: None,
        }
    }

    #[test]
    fn writes_frontmatter_only_file() {
        let (_tmp, cfg) = cfg();
        let tree = sample_tree("gmail:acct-1");
        let path = write_source_file(&cfg, &tree).unwrap();
        assert!(
            path.ends_with("raw/gmail-acct-1/_source.md"),
            "{}",
            path.display()
        );
        let body = fs::read_to_string(&path).unwrap();
        // Bracketed by frontmatter delimiters with no body after.
        assert!(body.starts_with("---\n"));
        assert!(body.trim_end().ends_with("---"));
        assert!(body.contains("tree_id: source:abc") || body.contains("tree_id: \"source:abc\""));
        assert!(body.contains("kind: source"));
        assert!(body.contains("status: active"));
        assert!(body.contains("last_sealed_at: null"));
    }

    #[test]
    fn rewrite_is_byte_identical_for_same_state() {
        let (_tmp, cfg) = cfg();
        let tree = sample_tree("slack:#eng");
        let path = write_source_file(&cfg, &tree).unwrap();
        let first = fs::read(&path).unwrap();
        write_source_file(&cfg, &tree).unwrap();
        let second = fs::read(&path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn updates_last_sealed_at_on_rewrite() {
        let (_tmp, cfg) = cfg();
        let mut tree = sample_tree("slack:#eng");
        write_source_file(&cfg, &tree).unwrap();
        tree.last_sealed_at = Some(Utc.timestamp_millis_opt(1_700_000_500_000).unwrap());
        tree.max_level = 3;
        let path = write_source_file(&cfg, &tree).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        assert!(body.contains("max_level: 3"));
        assert!(body.contains("last_sealed_at: 2023-11-14"), "{body}");
    }

    #[test]
    fn quotes_scalars_with_colons() {
        let (_tmp, cfg) = cfg();
        let tree = sample_tree("gmail:user@example.com");
        let path = write_source_file(&cfg, &tree).unwrap();
        let body = fs::read_to_string(&path).unwrap();
        // scope contains ':' → must be quoted to round-trip through YAML.
        assert!(body.contains("scope: \"gmail:user@example.com\""), "{body}");
    }
}
