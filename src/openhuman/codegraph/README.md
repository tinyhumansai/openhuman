# codegraph

Content-addressed code retrieval for coding subagents — the seed engine behind the issue-crusher / pr-reviewer skills. Given a checked-out git worktree, it indexes the tree's code files and answers "which files are most relevant to this query" by fusing a lexical (BM25) arm with a dense (structural-aug embedding) arm via reciprocal-rank fusion. Indexing is content-addressed: every file's `{BM25 tokens, structural-doc embedding}` is cached by its git **blob SHA** (+ embedding-model signature), and a branch's index is just a per-`(repo_id, ref)` **manifest** joined to that shared blob cache at query time — so a branch switch, new commit, or pull only (re)processes the blobs that actually changed. Pure Rust: git CLI for tree enumeration, `rusqlite` for storage, the `embeddings` domain (cloud-default) for vectors. No Python, no extra services.

## Responsibilities

- Enumerate tracked code files at a checkout (`git ls-files -s`, filtered to a fixed code-extension set, `≤ 100 KB`/file).
- Extract per-file content: identifier-split BM25 tokens (camelCase / snake_case → sub-words) and a heuristic "structural doc" (definition signatures + imports + called-symbol identifiers + leading doc/comments).
- Embed structural docs in batches (`≤ 128`/call) via the injected `EmbeddingProvider`, L2-normalising each vector.
- Cache `{tokens, emb}` by `(blob_sha, model)` in SQLite; rewrite the `(repo_id, ref)` manifest to the current tree (handles deletes/renames).
- Two index modes: `Lexical` (BM25-only, no embedder call) and `Dense` (structural-aug vectors + BM25). A size-gated `auto` picks between them.
- Search: hydrate the working set, rank with BM25-Okapi and cosine, RRF-fuse top-`PER_ARM` of each arm into top-`k`, and report a `Coverage` flag (`full`/`partial`/`none`).
- Expose `codegraph_index` and `codegraph_search` agent tools, workspace-sandboxed.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/codegraph/mod.rs` | Module docstring + `pub mod` decls + `pub use` re-exports. No logic. |
| `src/openhuman/codegraph/index.rs` | Indexing pipeline: tree enumeration (`tree_blobs`), `code_tokens`, `structural_doc`, `current_ref`, `count_code_files`, and the 3-phase `index_ref` (extract uncached blobs → batch-embed → persist + rewrite manifest). Defines `IndexMode`, `IndexReport`, `LEXICAL_MODEL`. |
| `src/openhuman/codegraph/search.rs` | Retrieval: `search_ref` hydrates the working set, runs `bm25_rank` + `dense_rank`, `rrf`-fuses, computes `Coverage`. Defines `SearchOutcome`, `Coverage`. |
| `src/openhuman/codegraph/store.rs` | SQLite persistence (`CodegraphStore`): `blob(sha,model,…)` content cache + `manifest(repo_id,git_ref,path,sha)`. `open`, `has_blob`, `put_blob(s)`, `set_manifest`, `hydrate`, `manifest_size`, `refs`, `gc`. Defines `BlobEntry`. |
| `src/openhuman/codegraph/tools.rs` | Agent tools `CodegraphIndexTool` / `CodegraphSearchTool` — arg parsing, workspace sandboxing, size-gated `auto` mode, provider/store wiring. |

## Public surface

Re-exported from `mod.rs`:

- **index**: `code_tokens`, `count_code_files`, `current_ref`, `index_ref`, `structural_doc`, `IndexMode` (`Lexical`/`Dense`), `IndexReport`, `LEXICAL_MODEL`.
- **search**: `search_ref`, `Coverage` (`Full`/`Partial`/`None`), `SearchOutcome` (`hits`, `coverage`, `indexed`, `total`).
- **store**: `CodegraphStore`, `BlobEntry` (`path`, `tokens`, `emb`).
- **tools**: `tools::{CodegraphIndexTool, CodegraphSearchTool}` (re-exported through `openhuman::tools::mod`).

## Agent tools

Owned in `tools.rs`, registered in `src/openhuman/tools/ops.rs`:

| Tool | Args | Returns |
| --- | --- | --- |
| `codegraph_index` | `path` (required), `ref?` (default current checkout), `mode?` (`auto`\|`lexical`\|`dense`) | `{mode, files, computed, cached, skipped}` |
| `codegraph_search` | `query` (required), `path` (required), `ref?`, `k?` (default 10) | `{hits:[paths], coverage:full\|partial\|none, indexed, total}` |

`codegraph_search` is index-first: if the `(repo, ref)` has no manifest, it auto-indexes synchronously (size-gated mode) before searching. Both tools canonicalize `path` and refuse any repo outside the workspace (`resolve_repo_dir`). `mode`/`auto` threshold is governed by `OPENHUMAN_CODEGRAPH_DENSE_MIN_FILES` (default 400 files → dense, else lexical).

## Persistence

SQLite DB at `<workspace_dir>/codegraph/index.db` (WAL journal, `synchronous=NORMAL` — a rebuildable cache). Two tables:

- `blob(sha, model, tokens, emb, dim)` PK `(sha, model)` — shared content cache: one row per unique file content per embedding-model key. `tokens` is the space-joined BM25 stream; `emb` is the L2-normalised vector as little-endian `f32` bytes. Shared across every repo/branch, so renames and unchanged files are free.
- `manifest(repo_id, git_ref, path, sha)` PK `(repo_id, git_ref, path)`, with index `manifest_repo_ref` — one row per file per branch/commit. A ref's index is its rows joined to `blob`. `gc()` drops blobs no live manifest references.

`repo_id` is the canonical worktree path; the `model` key is `LEXICAL_MODEL` (`codegraph:lexical:v1`) for lexical indexes or the embedding provider's `signature()` for dense.

## Dependencies

- `crate::openhuman::embeddings` — `EmbeddingProvider` trait (injected `&dyn`, unit-tested with fakes) for embedding structural docs/queries; `provider_from_config` builds the configured cloud-default provider in the tools. The provider's `signature()` is the blob-cache model key.
- `crate::openhuman::config::Config` — read by the tools to build the embedding provider.
- `crate::openhuman::tools::traits::{Tool, ToolResult}` — the agent-tool trait the two tools implement.
- External crates: `rusqlite` (+FTS5-era schema; current ranking is in-memory BM25, not FTS5), `anyhow`, `serde`/`serde_json`, `async_trait`, `tracing`. Git is shelled out via `std::process::Command` (no libgit2).

## Used by

- `src/openhuman/tools/mod.rs` re-exports the tools (`pub use crate::openhuman::codegraph::tools::*`); `src/openhuman/tools/ops.rs` constructs `CodegraphIndexTool` / `CodegraphSearchTool` into the agent tool registry.
- Registered as the domain `pub mod codegraph;` in `src/openhuman/mod.rs`.

## Notes / gotchas

- **No RPC controller / no event-bus subscriber / no config block of its own.** Surface is agent tools only; there is no `schemas.rs`, `rpc.rs`, or `bus.rs`. Behavior tunables come from env vars (`OPENHUMAN_CODEGRAPH_DENSE_MIN_FILES`), not the TOML config.
- The structural extractor is a **dependency-free heuristic**, not tree-sitter — the docstrings note a tree-sitter upgrade is intended to slot in behind `structural_doc` (the cached *content* would then change, so the `model` key isolates it).
- Empty-doc guard: a structure-less file (empty `__init__.py`, `x = 1`) would yield an empty structural doc, which cloud embedders reject (400 on empty input). `index_ref` falls back to the lexical tokens (or `"(no extractable content)"`) so embed inputs are never empty.
- `LEXICAL_MODEL` is deliberately a separate cache key from any embedder signature, so a later dense pass indexes fresh rather than colliding with embedding-less rows.
- BM25 runs **in-memory** over the hydrated per-repo working set (not SQLite FTS5) — the working set is one repo's files, kept small; the module docstring's "SQLite FTS5" framing describes the design lineage, the implemented lexical arm is `bm25_rank`.
- `Coverage` is `indexed / total` (manifest size): files whose blob isn't cached (skipped/oversized/pending) are omitted from `hydrate`, dropping coverage to `Partial`. On non-`Full` coverage the tool description tells the agent to treat hits as hints and also use grep.
- Search auto-detects the index mode: it first hydrates under the embedder signature (dense), and if empty falls back to the lexical key — lexical search makes no embedder round-trip.
- Several live/benchmark tests in `index.rs` are `#[ignore]` (need `OPENHUMAN_WORKSPACE` + a valid backend session, or `CODEGRAPH_BENCH_REPO`).
