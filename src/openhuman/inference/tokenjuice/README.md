# OpenHuman TokenJuice Adapter

The reusable compression engine ships as the separately released `tinyjuice`
TinyBus module. It is not linked into OpenHuman's dependency graph. This
directory is the host adapter and shared wire-contract layer.

OpenHuman-owned files:

| Path | Role |
| --- | --- |
| `mod.rs` | TinyBus calls, config installation, pass-through fallback, and savings wiring. |
| `types.rs` | Dependency-free copy of the stable JSON wire contract. |
| `schemas.rs` | JSON-RPC controller schemas and handlers. |
| `config_patch.rs` | Partial update shape for the `[tokenjuice]` config block. |
| `tools.rs` | OpenHuman agent tool implementation for `tokenjuice_retrieve`. |
| `ml/` | Bridge from TinyJuice's optional ML callback into `runtime_python_server` Kompress. |
| `savings.rs` | OpenHuman model-pricing attribution and persisted dashboard stats. |

TinyJuice-owned engine pieces:

| TinyJuice repository path | Role |
| --- | --- |
| `src/compress.rs` | Content router entry point. |
| `src/compressors/` | JSON, code, log, search, diff, HTML, ML slot, and generic compressors. |
| `src/cache/` | CCR store, retrieval markers, disk tier, ranged retrieval helpers. |
| `src/rules/` | Rule loader/compiler and embedded rule table. |
| `src/vendor/rules/*.json` | Vendored upstream rule JSON files. |
| `src/detect/`, `text/`, `tokens.rs`, `types.rs` | Detection, text helpers, token estimates, public types. |

Do not add the `tinyjuice` crate back to OpenHuman. Runtime services, settings
persistence, JSON-RPC, tools, pricing, and the optional ML callback stay here;
engine behavior stays behind the loadable module boundary.
