# content_store/

On-disk `.md` storage for chunk and summary bodies (Phase MD-content). SQLite holds `content_path` (relative, forward-slash) and `content_sha256` (over body bytes only) as pointers + integrity tokens; the body itself lives at `<content_root>/<content_path>`.

The body is **immutable** once written — only the YAML front-matter `tags:` block may be rewritten post-extraction.

## Files

- [`mod.rs`](mod.rs) — public surface: `StagedChunk`, `stage_chunks` (write all chunks atomically before SQLite upsert), `update_summary_tags` re-export.
- [`atomic.rs`](atomic.rs) — `write_if_new` (tempfile + fsync + rename, parent dir fsync on Unix), `stage_summary` (idempotent re-stage with on-disk SHA check + auto-rewrite on mismatch), `sha256_hex`, `StagedSummary`.
- [`compose.rs`](compose.rs) — YAML front-matter + body composition. `compose_chunk_file` for chunks (with email-only `participants:` / `aliases:` fields parsed from `gmail:{addr1|addr2|…}` source ids), `compose_summary_md` for summary nodes. `rewrite_tags` / `rewrite_summary_tags` swap the `tags:` block in place. `split_front_matter` parses `---\n…\n---\n<body>`.
- [`paths.rs`](paths.rs) — path generators. `chunk_rel_path` (`email/<participants_slug>/<id>.md`, `chat/<source_slug>/<id>.md`, `document/<source_slug>/<id>.md`); `summary_rel_path` (`summaries/{source,global,topic}/…`). `slugify_source_id` is the canonical filesystem-safe slug.
- [`read.rs`](read.rs) — `read_chunk_file` / `read_summary_file` parse front-matter and return body+SHA. `verify_*` compares against an expected SHA. `read_chunk_body` / `read_summary_body` resolve the path via SQLite and verify the integrity hash; this is the authoritative entry-point for callers that need the **full** body (LLM extractor, summariser, embedder, retrieval API).
- [`tags.rs`](tags.rs) — post-extraction tag rewrites. `update_chunk_tags` (atomic tempfile rewrite of the `tags:` block) and `update_summary_tags` (fetches entities from `mem_tree_entity_index`, builds Obsidian `kind/Value` tags, rewrites, verifies body SHA is unchanged). `slugify_tag_kind`, `slugify_tag_value`, `entity_tag` build the tag strings.

## Integrity contract

The body bytes never change after the first write. The SHA-256 stored in SQLite is computed over body bytes only — front-matter (including `tags:`) can be rewritten without invalidating the hash. Read paths verify SHA on every fetch and fail loudly on mismatch rather than serve corrupt data into the extractor or summariser.
