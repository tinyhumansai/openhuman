# OpenHuman TokenJuice Adapter

The reusable compression engine now lives in the vendored `tinyjuice` crate at
`vendor/tinyjuice` and is patched through Cargo. This directory is the
OpenHuman adapter layer.

OpenHuman-owned files:

| Path | Role |
| --- | --- |
| `mod.rs` | Stable OpenHuman module seam, TinyJuice re-exports, config-to-engine install hook, ML/savings callback wiring. |
| `schemas.rs` | JSON-RPC controller schemas and handlers. |
| `config_patch.rs` | Partial update shape for the `[tokenjuice]` config block. |
| `tools.rs` | OpenHuman agent tool implementation for `tokenjuice_retrieve`. |
| `ml/` | Bridge from TinyJuice's optional ML callback into `runtime_python_server` Kompress. |
| `savings.rs` | OpenHuman model-pricing attribution and persisted dashboard stats. |

TinyJuice-owned engine pieces:

| TinyJuice path | Role |
| --- | --- |
| `vendor/tinyjuice/src/compress.rs` | Content router entry point. |
| `vendor/tinyjuice/src/compressors/` | JSON, code, log, search, diff, HTML, ML slot, and generic compressors. |
| `vendor/tinyjuice/src/cache/` | CCR store, retrieval markers, disk tier, ranged retrieval helpers. |
| `vendor/tinyjuice/src/rules/` | Rule loader/compiler and embedded rule table. |
| `vendor/tinyjuice/src/vendor/rules/*.json` | Vendored upstream rule JSON files. |
| `vendor/tinyjuice/src/detect/`, `text/`, `tokens.rs`, `types.rs` | Detection, text helpers, token estimates, public types. |

Do not add OpenHuman runtime dependencies to TinyJuice. Runtime services,
settings persistence, JSON-RPC, tools, and pricing stay in this adapter.
