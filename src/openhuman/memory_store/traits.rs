//! Storage compatibility traits.
//!
//! Every stored memory kind must answer two questions:
//!
//! 1. **Can it be embedded into a vector?** — yes, via [`VectorEmbeddable`].
//!    The trait provides the canonical embeddable string for the object so a
//!    single embedding pipeline can index any kind uniformly.
//! 2. **Can it be represented as an Obsidian-compatible markdown file?** —
//!    yes, via [`ObsidianRepresentable`]. The trait yields a relative vault
//!    path and a fully-formed markdown body (YAML front-matter + content)
//!    that can be written into the content store and opened by Obsidian
//!    without further processing.
//!
//! Together these two traits are the contract that makes "everything in
//! memory_store is vector and obsidian compatible" a checkable property
//! rather than a slogan — the compiler enforces it for every new storage
//! kind that gets added.

use std::path::PathBuf;

use crate::openhuman::memory_store::chunks::types::Chunk;
use crate::openhuman::memory_store::contacts::Person;
use crate::openhuman::memory_store::kinds::MemoryKind;
use crate::openhuman::memory_store::trees::{SummaryNode, Tree};

/// A rendered Obsidian markdown file: where it lives in the vault and what
/// bytes to write. Vault path is relative to the content-store root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObsidianFile {
    pub relative_path: PathBuf,
    pub markdown: String,
}

/// Objects that can produce a canonical string for embedding into the
/// vector store. The returned text should be deterministic and stable across
/// calls so re-embedding produces consistent vectors.
pub trait VectorEmbeddable {
    /// The MemoryKind this value belongs to. Used by the embedding pipeline
    /// to route vectors into per-kind namespaces.
    fn memory_kind(&self) -> MemoryKind;

    /// Canonical UTF-8 text fed to the embedding model. Strip front-matter,
    /// markdown formatting noise, and anything not semantically meaningful.
    fn embeddable_text(&self) -> String;
}

/// Objects that can be rendered as an Obsidian-compatible markdown file.
/// The file should round-trip through the content store unchanged so vault
/// edits stay idempotent.
pub trait ObsidianRepresentable {
    fn to_obsidian(&self) -> ObsidianFile;
}

// ---- impls: Chunk ----------------------------------------------------------

impl VectorEmbeddable for Chunk {
    fn memory_kind(&self) -> MemoryKind {
        MemoryKind::Chunk
    }

    fn embeddable_text(&self) -> String {
        self.content.clone()
    }
}

impl ObsidianRepresentable for Chunk {
    fn to_obsidian(&self) -> ObsidianFile {
        let tags_yaml = if self.metadata.tags.is_empty() {
            String::new()
        } else {
            let lines: Vec<String> = self
                .metadata
                .tags
                .iter()
                .map(|t| format!("  - {}", t))
                .collect();
            format!("tags:\n{}\n", lines.join("\n"))
        };
        let markdown = format!(
            "---\nid: {}\nsource_kind: {}\nsource_id: {}\nseq: {}\n{}---\n\n{}\n",
            self.id,
            self.metadata.source_kind.as_str(),
            self.metadata.source_id,
            self.seq_in_source,
            tags_yaml,
            self.content
        );
        ObsidianFile {
            relative_path: PathBuf::from("chunks").join(format!("{}.md", self.id)),
            markdown,
        }
    }
}

// ---- impls: Tree + SummaryNode --------------------------------------------

impl VectorEmbeddable for SummaryNode {
    fn memory_kind(&self) -> MemoryKind {
        MemoryKind::Tree
    }

    fn embeddable_text(&self) -> String {
        self.content.clone()
    }
}

impl ObsidianRepresentable for SummaryNode {
    fn to_obsidian(&self) -> ObsidianFile {
        let markdown = format!(
            "---\nid: {}\ntree_id: {}\nlevel: {}\n---\n\n{}\n",
            self.id, self.tree_id, self.level, self.content
        );
        ObsidianFile {
            relative_path: PathBuf::from("summaries").join(format!("{}.md", self.id)),
            markdown,
        }
    }
}

impl ObsidianRepresentable for Tree {
    fn to_obsidian(&self) -> ObsidianFile {
        let markdown = format!(
            "---\nid: {}\nkind: {:?}\nstatus: {:?}\n---\n\nTree {} ({:?})\n",
            self.id, self.kind, self.status, self.id, self.kind
        );
        ObsidianFile {
            relative_path: PathBuf::from("trees").join(format!("{}.md", self.id)),
            markdown,
        }
    }
}

// ---- impls: Contact (Person) ----------------------------------------------

impl VectorEmbeddable for Person {
    fn memory_kind(&self) -> MemoryKind {
        MemoryKind::Contact
    }

    fn embeddable_text(&self) -> String {
        // Embed the display name plus primary email — both carry useful
        // disambiguation signal. Handles are routing keys, not semantic
        // content, and intentionally excluded.
        let mut parts: Vec<String> = Vec::new();
        if let Some(name) = self.display_name.as_deref() {
            parts.push(name.to_string());
        }
        if let Some(email) = self.primary_email.as_deref() {
            parts.push(email.to_string());
        }
        parts.join("\n")
    }
}

impl ObsidianRepresentable for Person {
    fn to_obsidian(&self) -> ObsidianFile {
        let display = self.display_name.as_deref().unwrap_or("Unknown");
        let email = self.primary_email.as_deref().unwrap_or("");
        let markdown = format!(
            "---\nperson_id: {}\n---\n\n# {}\n\nEmail: {}\n",
            self.id, display, email
        );
        ObsidianFile {
            relative_path: PathBuf::from("contacts").join(format!("{}.md", self.id)),
            markdown,
        }
    }
}

// Documents are no longer a first-class MemoryKind — the md backend
// (`content/`) is the canonical persistence for any document body. Anything
// that historically used `StoredMemoryDocument` should land its body as a
// raw md file and reference it via path.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_store::chunks::types::{Metadata, SourceKind};
    use chrono::Utc;

    fn sample_chunk() -> Chunk {
        let ts = Utc::now();
        Chunk {
            id: "chunk-1".into(),
            content: "hello world".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                timestamp: ts,
                time_range: (ts, ts),
                owner: "alice".into(),
                source_ref: None,
                tags: vec!["person:alice".into()],
            },
            seq_in_source: 7,
            token_count: 2,
            created_at: ts,
            partial_message: false,
        }
    }

    #[test]
    fn chunk_traits_render_expected_kind_and_obsidian_path() {
        let chunk = sample_chunk();
        assert_eq!(chunk.memory_kind(), MemoryKind::Chunk);
        assert_eq!(chunk.embeddable_text(), "hello world");

        let obsidian = chunk.to_obsidian();
        assert_eq!(obsidian.relative_path, PathBuf::from("chunks/chunk-1.md"));
        assert!(obsidian.markdown.contains("source_kind: chat"));
        assert!(obsidian.markdown.contains("source_id: slack:#eng"));
        assert!(obsidian.markdown.contains("hello world"));
    }

    #[test]
    fn summary_node_traits_render_expected_kind_and_path() {
        let node = SummaryNode {
            id: "summary-1".into(),
            tree_id: "tree-1".into(),
            tree_kind: crate::openhuman::memory_store::trees::TreeKind::Source,
            level: 1,
            parent_id: None,
            child_ids: vec!["chunk-1".into()],
            content: "summary body".into(),
            token_count: 3,
            entities: vec![],
            topics: vec![],
            time_range_start: Utc::now(),
            time_range_end: Utc::now(),
            score: 0.5,
            sealed_at: Utc::now(),
            deleted: false,
            embedding: None,
        };
        assert_eq!(node.memory_kind(), MemoryKind::Tree);
        assert_eq!(node.embeddable_text(), "summary body");
        let obsidian = node.to_obsidian();
        assert_eq!(
            obsidian.relative_path,
            PathBuf::from("summaries/summary-1.md")
        );
        assert!(obsidian.markdown.contains("tree_id: tree-1"));
        assert!(obsidian.markdown.contains("summary body"));
    }

    #[test]
    fn tree_traits_render_obsidian_metadata() {
        let tree = Tree {
            id: "tree-1".into(),
            kind: crate::openhuman::memory_store::trees::TreeKind::Topic,
            scope: "topic:phoenix".into(),
            root_id: Some("summary-root".into()),
            max_level: 2,
            status: crate::openhuman::memory_store::trees::TreeStatus::Active,
            created_at: Utc::now(),
            last_sealed_at: None,
        };
        let obsidian = tree.to_obsidian();
        assert_eq!(obsidian.relative_path, PathBuf::from("trees/tree-1.md"));
        assert!(obsidian.markdown.contains("id: tree-1"));
        assert!(obsidian.markdown.contains("Tree tree-1"));
        assert!(obsidian.markdown.contains("Topic"));
    }

    #[test]
    fn person_traits_render_name_and_email_when_present() {
        let now = Utc::now();
        let person = Person {
            id: crate::openhuman::people::types::PersonId::new(),
            display_name: Some("Alice Example".into()),
            primary_email: Some("alice@example.com".into()),
            primary_phone: Some("+1 555 0100".into()),
            handles: vec![
                crate::openhuman::people::types::Handle::DisplayName("Alice Example".into()),
                crate::openhuman::people::types::Handle::Email("alice@example.com".into()),
            ],
            created_at: now,
            updated_at: now,
        };
        assert_eq!(person.memory_kind(), MemoryKind::Contact);
        assert_eq!(person.embeddable_text(), "Alice Example\nalice@example.com");
        let obsidian = person.to_obsidian();
        assert_eq!(
            obsidian.relative_path,
            PathBuf::from("contacts").join(format!("{}.md", person.id))
        );
        assert!(obsidian.markdown.contains("# Alice Example"));
        assert!(obsidian.markdown.contains("Email: alice@example.com"));
    }

    #[test]
    fn person_traits_fall_back_when_fields_are_missing() {
        let now = Utc::now();
        let person = Person {
            id: crate::openhuman::people::types::PersonId::new(),
            display_name: None,
            primary_email: None,
            primary_phone: None,
            handles: vec![],
            created_at: now,
            updated_at: now,
        };
        assert_eq!(person.embeddable_text(), "");
        let obsidian = person.to_obsidian();
        assert!(obsidian.markdown.contains("# Unknown"));
        assert!(obsidian.markdown.contains("Email: "));
    }
}
