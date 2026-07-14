//! On-disk format parity — Layer-1 regression pins (migration W3 gate, spec §0.3).
//!
//! Existing user workspaces must open unchanged after the store cutover. These
//! are the cheap, fixture-free asserters from the parity checklist: they pin the
//! crate's deterministic **on-disk contracts** to the exact byte forms that
//! historical OpenHuman workspaces were written with, so any future crate change
//! that would silently reshape chunk IDs, vector encoding, or vault paths fails
//! here instead of corrupting a real workspace.
//!
//! The golden constants were computed from the format spec (SHA-256 first-32-hex
//! chunk IDs; little-endian packed f32 vectors) and cross-checked against the
//! crate at the W3 baseline. The Layer-2 golden-workspace differential harness
//! (a real `chunks.db` + vault opened and compared) is the merge gate for the
//! actual store flips; this layer runs on every PR.
//!
//! Test-only module — no runtime code.

#[cfg(test)]
mod tests {
    use tinycortex::memory::chunks::{chunk_id, SourceKind};
    use tinycortex::memory::store::content::chunk_rel_path;
    use tinycortex::memory::store::vectors::{bytes_to_vec, vec_to_bytes};

    /// P1 — the deterministic chunk ID is SHA-256 over
    /// `source_kind \0 source_id \0 seq_be \0 content`, first 32 hex chars.
    /// This golden is the value historical workspaces indexed by; a change to
    /// the hash inputs / order / separators would strand every existing chunk.
    #[test]
    fn chunk_id_matches_historical_golden() {
        let id = chunk_id(SourceKind::Document, "src-1", 5, "hello world");
        assert_eq!(id, "2be5fac18b12bfb417736b54deaf5f9d");
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// P1 — every field participates in the hash, and `seq` is order-sensitive.
    /// Guards against an input being dropped or reordered (which a property-free
    /// golden alone would miss on a symmetric swap).
    #[test]
    fn chunk_id_is_sensitive_to_every_field() {
        let base = chunk_id(SourceKind::Document, "src-1", 5, "hello world");
        assert_ne!(base, chunk_id(SourceKind::Chat, "src-1", 5, "hello world"));
        assert_ne!(
            base,
            chunk_id(SourceKind::Document, "src-2", 5, "hello world")
        );
        assert_ne!(
            base,
            chunk_id(SourceKind::Document, "src-1", 6, "hello world")
        );
        assert_ne!(
            base,
            chunk_id(SourceKind::Document, "src-1", 5, "hello worlds")
        );
        // Determinism: same inputs, same id.
        assert_eq!(
            base,
            chunk_id(SourceKind::Document, "src-1", 5, "hello world")
        );
    }

    /// P2 — vectors persist as little-endian packed f32, 4 bytes/element, no
    /// header. The golden byte string is what existing `vectors.embedding`
    /// BLOBs and `mem_tree_*_embeddings` sidecars were written with.
    #[test]
    fn vector_encoding_is_le_packed_f32() {
        let v = vec![1.0f32, -2.0, 0.5];
        let bytes = vec_to_bytes(&v);
        assert_eq!(bytes.len(), v.len() * 4);
        assert_eq!(hex(&bytes), "0000803f000000c00000003f");
        // Round-trips exactly.
        assert_eq!(bytes_to_vec(&bytes), v);
    }

    /// P6 — vault paths sanitize IDs to cross-platform-safe filenames. Chunk IDs
    /// contain colons (`chat:slack:#eng:0`) that are illegal on Windows NTFS;
    /// the path must not leak them, and must be deterministic so an existing
    /// vault file is found in place.
    #[test]
    fn content_paths_are_windows_safe_and_stable() {
        let p1 = chunk_rel_path("chat", "slack:#eng", "chat:slack:#eng:0");
        let p2 = chunk_rel_path("chat", "slack:#eng", "chat:slack:#eng:0");
        assert_eq!(p1, p2, "path derivation must be deterministic");
        assert!(
            !p1.contains(':'),
            "path must not contain Windows-illegal ':' -> {p1}"
        );
        assert!(p1.ends_with(".md"), "chunk files are markdown -> {p1}");
    }

    /// P6 (differential) — the host and crate `chunk_rel_path` must produce
    /// **byte-identical** vault paths for every id shape a real workspace holds.
    /// Both impls still exist (content is not flipped until W3), so a crate-side
    /// change to `slugify_source_id` / `sanitize_filename` / the email special
    /// case would silently strand every existing chunk file under a new path.
    /// This pins them together over an adversarial corpus (colons, all
    /// Windows-illegal chars, unicode, >255-char ids, gmail participant slugs,
    /// malformed email source_ids) so any drift fails here, not on a user's disk.
    #[test]
    fn chunk_rel_path_host_crate_byte_parity() {
        use crate::openhuman::memory_store::content::paths as host;
        use tinycortex::memory::store::content as cortex;

        let long_id = "x".repeat(300);
        let corpus: &[(&str, &str, &str)] = &[
            // (source_kind, source_id, chunk_id)
            ("chat", "slack:#eng", "chat:slack:#eng:0"),
            ("chat", "Slack:#Eng__Team", "chat:slack:#eng:0"),
            ("document", "file:///Users/x/Notes.md", "doc:notes:3"),
            ("document", "weird__source__id", "id-with-no-illegal-chars"),
            ("chat", "src", "a\\b/c:d*e?f\"g<h>i|j"),
            ("chat", "东京:room", "chat:东京:0"),
            ("chat", "src", &long_id),
            // Email: well-formed gmail participants → one slugified folder.
            (
                "email",
                "gmail:notifications@github.com|sanil@x.com",
                "email:msg:0",
            ),
            ("email", "gmail:Alice@X.com|bob@y.com", "email:msg:1"),
            // Email: malformed / legacy source_id → flat fallback layout.
            ("email", "legacyid", "email:legacy:0"),
            ("email", "gmail:", "email:empty-participants:0"),
        ];

        for (kind, source_id, chunk_id) in corpus {
            let h = host::chunk_rel_path(kind, source_id, chunk_id);
            let c = cortex::chunk_rel_path(kind, source_id, chunk_id);
            assert_eq!(
                h, c,
                "chunk_rel_path diverged for (kind={kind}, source_id={source_id}, chunk_id={chunk_id}): host={h} crate={c}"
            );
            assert!(!h.contains(':'), "host path leaked ':' -> {h}");
            assert!(h.ends_with(".md"), "chunk files are markdown -> {h}");
        }
    }

    /// P6 (differential) — the same byte-parity requirement for summary paths.
    /// The summary basename (`summary_filename`) and the `wiki/summaries/...`
    /// layout per `SummaryTreeKind` must match across host and crate, or a
    /// re-open would not find an existing sealed summary in place.
    #[test]
    fn summary_rel_path_host_crate_byte_parity() {
        use crate::openhuman::memory_store::content::paths as host;
        use tinycortex::memory::store::content as cortex;

        // (host kind, crate kind, scope_slug) — variants are 1:1 across sides.
        let kinds = [
            (
                host::SummaryTreeKind::Source,
                cortex::SummaryTreeKind::Source,
                "source-slug",
            ),
            (
                host::SummaryTreeKind::Global,
                cortex::SummaryTreeKind::Global,
                "ignored-for-global",
            ),
            (
                host::SummaryTreeKind::Topic,
                cortex::SummaryTreeKind::Topic,
                "phoenix-migration",
            ),
        ];
        // Canonical ms-first ids, legacy level-first ids, and malformed shapes
        // that must fall back through `sanitize_filename` on both sides.
        let summary_ids: &[&str] = &[
            "summary:1700000000000:L2-abc-uuid",
            "summary:L3:legacy-uuid",
            "summary:1700000000000:L2-a/b", // illegal tail → sanitized
            "summary:notms:L1-tail",        // non-13-digit ms → fallback
            "raw-unknown-shape:with:colons", // unknown → sanitize_filename
            "东京-summary",                 // unicode
        ];

        for (hk, ck, scope) in kinds {
            for level in [0u32, 1, 4] {
                for sid in summary_ids {
                    let h = host::summary_rel_path(hk, scope, level, sid);
                    let c = cortex::summary_rel_path(ck, scope, level, sid);
                    assert_eq!(
                        h, c,
                        "summary_rel_path diverged for (scope={scope}, level={level}, id={sid}): host={h} crate={c}"
                    );
                    assert!(!h.contains(':'), "host summary path leaked ':' -> {h}");
                }
            }
        }
    }

    /// P10 — the embedding-space **signature** string that keys every persisted
    /// vector. Host (`embeddings::format_embedding_signature`) and crate
    /// (`store::vectors::format_embedding_signature`) each own their **own** copy
    /// of this formatter, so a change to either would silently split one
    /// embedding space into two — every existing vector would look stale under
    /// the new signature and trigger a full re-embed storm on the next open.
    /// Pin both to the golden `provider={name};model={model};dims={dims}` form
    /// over a corpus (real provider triples plus empties / special chars).
    #[test]
    fn embedding_signature_host_crate_byte_parity() {
        use crate::openhuman::embeddings::format_embedding_signature as host_sig;
        use tinycortex::memory::store::vectors::format_embedding_signature as cortex_sig;

        // (name, model_id, dims, expected golden)
        let corpus: &[(&str, &str, usize, &str)] = &[
            (
                "voyage",
                "voyage-3",
                1024,
                "provider=voyage;model=voyage-3;dims=1024",
            ),
            (
                "openai",
                "text-embedding-3-small",
                1536,
                "provider=openai;model=text-embedding-3-small;dims=1536",
            ),
            (
                "ollama",
                "nomic-embed-text",
                768,
                "provider=ollama;model=nomic-embed-text;dims=768",
            ),
            (
                "cohere",
                "embed-english-v3.0",
                1024,
                "provider=cohere;model=embed-english-v3.0;dims=1024",
            ),
            ("inert", "none", 0, "provider=inert;model=none;dims=0"),
            // Edge shapes: empty model, punctuation in model id.
            ("noop", "", 3, "provider=noop;model=;dims=3"),
            ("x", "m-1_2.3", 42, "provider=x;model=m-1_2.3;dims=42"),
        ];

        for (name, model, dims, golden) in corpus {
            let h = host_sig(name, model, *dims);
            let c = cortex_sig(name, model, *dims);
            assert_eq!(
                h, c,
                "signature diverged for (name={name}, model={model}, dims={dims}): host={h} crate={c}"
            );
            assert_eq!(&h, golden, "signature format drifted from the golden form");
        }
    }

    fn hex(bytes: &[u8]) -> String {
        use std::fmt::Write;
        bytes
            .iter()
            .fold(String::with_capacity(bytes.len() * 2), |mut acc, b| {
                let _ = write!(acc, "{b:02x}");
                acc
            })
    }
}
