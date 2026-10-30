---
description: >-
  Bounded, read-only source scanning, conservative dependency evidence and
  hash-anchored verification through the host filesystem security policy.
icon: scan-search
---

# Workspace source evidence

`source_evidence` is a local, read-only agent tool for inspecting **the current authorized workspace**. It does not make network requests, create a persistent index, run source code, or grant additional filesystem access. Its outputs may, however, be included in requests to the model serving the current turn, so the host's security policy remains authoritative.

This is **evidence**, not a compiler, language server or full-project static-analysis index. A result only describes source files actually observed during one bounded request.

## Operations

| Operation | Input | Result |
| --- | --- | --- |
| `scan` (default) | `path`, optional literal `query` | Observed file paths and bounded, line-numbered excerpts, with hashes and completeness flags |
| `locate` | Nonempty `query`, optional `path` | Ranked candidate files and explainable match evidence, not a claim that missing files are irrelevant |
| `graph` | Optional `path`, `max_edges`, `focus_path` | A conservative projection of local imports and supported dependencies, with ambiguity/completeness indicators |
| `verify` | A file `path` and `expected_sha256`, optional line range | A fresh excerpt only if the current file bytes match the expected content digest |

The tool uses the existing `tinytools::Tool` contract and `PermissionLevel::ReadOnly` and appears in the host filesystem tool collection.

Example scan arguments:

```json
{"operation":"scan","path":"src","query":"create_chat_model","max_files":10}
```

Example location request:

```json
{"operation":"locate","path":"src","query":"where is model routing configured","max_results":5}
```

Example verification request (replace the example hash with a real returned source hash):

```json
{"operation":"verify","path":"src/main.rs","expected_sha256":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef","start_line":1,"end_line":25}
```

## Scope and resource limits

- Access is bounded to the active workspace, using the same host `SecurityPolicy`. Every candidate file is authorized separately; a directory check alone does not grant child access.
- Explicit symlinks, traversal outside the authorized root, credential-like filenames and hidden secret directories are excluded. A file opened after an initial check is checked again before returning evidence.
- Maximum **32 files** per call, **256 KiB** per file, **2 MiB** total observed source bytes, **8,192** traversal entries, and depth **12**. Results are bounded independently of file-reading limits.
- Graph mode returns at most **64 edges**. Locate returns at most **5** candidates. Verification returns at most **80 lines**.
- `query` is a **literal** limited to 128 bytes, not an arbitrary regular-expression or shell command. Argument fields are closed to known names; invalid or unexpected fields fail rather than silently widening a scan.
- The result reports `catalog_complete`, `truncated_reason`, `next_after_path` and/or `snapshot_atomic` where relevant. `snapshot_atomic:false` means the result is **not an atomic repository snapshot**.

These caps bound resource consumption; they do not guarantee that the first page contains every relevant file. A result truncated by entries, files, bytes or output size must not be treated as proof that no other sources exist.

## Conservative dependency semantics

The graph recognizes selected Rust, JavaScript/TypeScript and Python syntactic patterns using an intentionally conservative lexical scan. It is not a full language parser. The tool does not assert edges when the target cannot be resolved uniquely from the **observed** file catalog.

When the catalog is incomplete, purported matches are reported as provisional candidates rather than confirmed edges. Reverse-impact claims are omitted when their prerequisite edges or coverage are truncated. Missing links mean **unknown**, not **no dependency**.

For authoritative refactor decisions, supplement this evidence with the owning language toolchain, workspace tests and actual dynamic-call documentation.

## Reproducibility and limits of verification

Each returned excerpt carries a source content hash. Re-read a file using `verify` and its actual `expected_sha256` before citing the excerpt as representing the current source revision. A matching hash verifies file content at the instant of that read, **not** the atomicity of a multi-file repository checkout or the success of a build/test.

The source-evidence implementation and its tests live in:

- `crates/openhuman-core/src/tools/impl/filesystem/source_evidence.rs`
- `source_evidence_args.rs`, `source_locate.rs`, `source_graph.rs`
- Sibling `*_tests.rs` modules covering unauthorized paths, symlink handling, request shapes, truncated scans, dependency ambiguity and source revisions

Run the focused Rust tests with `cargo test -p openhuman --lib source_evidence` and `cargo test -p openhuman --lib source_graph`. The broader repository CI and SecurityPolicy tests must also pass before publication.
