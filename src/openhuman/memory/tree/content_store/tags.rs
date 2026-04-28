//! Post-extraction tag rewriting for chunk `.md` files.
//!
//! After the LLM extraction job runs, it produces a list of entities. Each
//! entity is converted to an Obsidian-style hierarchical tag (`kind/Value`)
//! and written into the `tags:` block in the chunk's front-matter.
//!
//! The body bytes (and therefore the SHA-256) are never changed — only the
//! front-matter is rewritten.

use std::path::Path;

use super::compose::rewrite_tags;

/// Rewrite the `tags:` block in a chunk's on-disk `.md` file.
///
/// `abs_path` — absolute path to the chunk file.
/// `tags`     — new list of tag strings (Obsidian `kind/Value` format).
///
/// The operation is atomic: the new file is written to a sibling temp path and
/// then renamed over the original. If the file does not exist, the call is a
/// no-op (returns `Ok(())`).
///
/// Note: unlike the initial chunk write, tag rewrites MAY overwrite an
/// existing file. The immutability contract covers the **body** only; tags are
/// explicitly designed to be updated post-extraction.
pub fn update_chunk_tags(abs_path: &Path, tags: &[String]) -> anyhow::Result<()> {
    if !abs_path.exists() {
        log::debug!(
            "[content_store::tags] skipping tag update — file not found: {}",
            abs_path.display()
        );
        return Ok(());
    }

    let old_bytes =
        std::fs::read(abs_path).map_err(|e| anyhow::anyhow!("read {:?}: {e}", abs_path))?;

    let new_bytes = rewrite_tags(&old_bytes, tags)
        .map_err(|e| anyhow::anyhow!("rewrite_tags {:?}: {e}", abs_path))?;

    // Write the new content atomically via a sibling temp file.
    let parent = abs_path.parent().unwrap_or_else(|| Path::new("."));
    let tmp_name = format!(".tmp_tags_{}.md", crate_temp_id());
    let tmp_path = parent.join(&tmp_name);

    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp_path)
            .map_err(|e| anyhow::anyhow!("create tag-rewrite tempfile {:?}: {e}", tmp_path))?;
        f.write_all(&new_bytes)
            .map_err(|e| anyhow::anyhow!("write tag-rewrite tempfile {:?}: {e}", tmp_path))?;
        f.sync_all()
            .map_err(|e| anyhow::anyhow!("fsync tag-rewrite tempfile {:?}: {e}", tmp_path))?;
    }

    std::fs::rename(&tmp_path, abs_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        anyhow::anyhow!("rename tag-rewrite {:?} -> {:?}: {e}", tmp_path, abs_path)
    })?;

    log::debug!(
        "[content_store::tags] updated tags in {}",
        abs_path.display()
    );
    Ok(())
}

/// Slugify an entity kind string for use in an Obsidian hierarchical tag.
///
/// Output: lowercase, spaces and non-alphanumeric chars replaced with `-`,
/// consecutive dashes collapsed, leading/trailing dashes stripped.
///
/// Example: `"Person"` → `"person"`, `"GitHub Repo"` → `"github-repo"`
pub fn slugify_tag_kind(kind: &str) -> String {
    slugify_tag_component(kind)
}

/// Slugify an entity value string for use in an Obsidian hierarchical tag.
///
/// Like `slugify_tag_kind`, but capitalises the first letter of each word
/// so values are visually distinct from kinds:
///
/// `"alice johnson"` → `"Alice-Johnson"`,
/// `"project Phoenix"` → `"Project-Phoenix"`
pub fn slugify_tag_value(value: &str) -> String {
    // Split on non-alphanumeric boundaries, capitalise first letter of each word.
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();

    for ch in value.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            current.push(ch);
        } else {
            if !current.is_empty() {
                parts.push(capitalise(&current));
                current.clear();
            }
        }
    }
    if !current.is_empty() {
        parts.push(capitalise(&current));
    }

    let joined = parts.join("-");
    if joined.is_empty() {
        "unknown".to_string()
    } else {
        joined
    }
}

/// Build an Obsidian-style `kind/Value` tag string from raw entity kind + surface.
pub fn entity_tag(kind: &str, surface: &str) -> String {
    format!("{}/{}", slugify_tag_kind(kind), slugify_tag_value(surface))
}

fn slugify_tag_component(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut out = String::new();
    let mut last_dash = true;
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_end_matches('-');
    if trimmed.is_empty() {
        "unknown".to_string()
    } else {
        trimmed.to_string()
    }
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            upper + chars.as_str()
        }
    }
}

fn crate_temp_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    format!("{ns:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::content_store::atomic::write_if_new;
    use crate::openhuman::memory::tree::content_store::compose::compose_chunk_file;
    use crate::openhuman::memory::tree::types::{Chunk, Metadata, SourceKind};
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn sample_chunk() -> Chunk {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        Chunk {
            id: "tags_test".into(),
            content: "hello from tags test".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec!["old/Tag".into()],
                source_ref: None,
            },
            token_count: 4,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        }
    }

    #[test]
    fn update_chunk_tags_replaces_tag_block() {
        let dir = TempDir::new().unwrap();
        let chunk = sample_chunk();
        let (full, _) = compose_chunk_file(&chunk);
        let path = dir.path().join("0.md");
        write_if_new(&path, &full).unwrap();

        update_chunk_tags(
            &path,
            &["person/Alice-Smith".into(), "project/Phoenix".into()],
        )
        .unwrap();

        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("  - person/Alice-Smith"));
        assert!(updated.contains("  - project/Phoenix"));
        assert!(!updated.contains("  - old/Tag"));
        // Body unchanged.
        assert!(updated.ends_with("hello from tags test"));
    }

    #[test]
    fn update_chunk_tags_is_noop_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.md");
        assert!(update_chunk_tags(&path, &["p/X".into()]).is_ok());
    }

    #[test]
    fn slugify_tag_kind_examples() {
        assert_eq!(slugify_tag_kind("Person"), "person");
        assert_eq!(slugify_tag_kind("GitHub Repo"), "github-repo");
        assert_eq!(slugify_tag_kind("EMAIL"), "email");
    }

    #[test]
    fn slugify_tag_value_capitalises_words() {
        assert_eq!(slugify_tag_value("alice johnson"), "Alice-Johnson");
        assert_eq!(slugify_tag_value("project Phoenix"), "Project-Phoenix");
        assert_eq!(slugify_tag_value("OPENAI"), "OPENAI");
    }

    #[test]
    fn entity_tag_builds_obsidian_tag() {
        assert_eq!(
            entity_tag("person", "Alice Johnson"),
            "person/Alice-Johnson"
        );
        assert_eq!(entity_tag("ORG", "Tinyhumans AI"), "org/Tinyhumans-AI");
    }
}
