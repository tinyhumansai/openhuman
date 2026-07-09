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
