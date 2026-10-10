# about_app

The single source of truth for the OpenHuman desktop app's **user-facing capability catalog**. It enumerates every capability the app exposes to end users: what each one does, where it lives in the UI (`how_to`), its maturity (`stable` / `beta` / `coming_soon` / `deprecated`), and a per-capability privacy disclosure (what data, if any, leaves the device and where it goes). The catalog is a compile-time static table; the module exposes read-only list / lookup / search over it via JSON-RPC. It is stateless: no persistence, no event subscribers, no agent tools.

## Responsibilities

- Define the canonical, hard-coded list of user-facing capabilities (`CAPABILITIES` in [`catalog.rs`](./catalog.rs)).
- Classify each capability by `CapabilityCategory` (conversation, intelligence, workflows, local_ai, team, settings, auth, channels, automation, mobile) and `CapabilityStatus`.
- Attach optional `CapabilityPrivacy` disclosures (`leaves_device`, `data_kind`, `destinations`) so the in-app Privacy surface can render "what leaves my computer".
- Provide read APIs: list all (optionally filtered by category), look up one by stable id, keyword search across id/name/domain/category/description/how_to/status.
- Validate catalog integrity at first access (no empty ids, no duplicate ids) via a `OnceLock` guard.
- Expose those reads to CLI + JSON-RPC through controller schemas.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/platform/about_app/mod.rs` | Export-only module root + docstring. Re-exports catalog reads, ops entry points, schema registry hooks, and types. |
| `crates/openhuman-core/src/platform/about_app/types.rs` | Serde domain types: `Capability`, `CapabilityCategory` (with `as_str` / `FromStr` incl. aliases), `CapabilityStatus`, `CapabilityPrivacy`, `PrivacyDataKind`. Inline serde/roundtrip tests. |
| `crates/openhuman-core/src/platform/about_app/catalog.rs` | Read API over the `CAPABILITIES` data it includes from [`catalog_data.rs`](./catalog_data.rs). Implements `all_capabilities`, `capabilities_by_category`, `lookup`, `search`, and the `ensure_validated` integrity check. |
| `crates/openhuman-core/src/platform/about_app/ops.rs` | RPC-facing logic returning `Outcome<T>`: `list_capabilities`, `lookup_capability`, `search_capabilities`. Thin wrappers over [`catalog.rs`](./catalog.rs) with summary logs. |
| `crates/openhuman-core/src/platform/about_app/schemas.rs` | Controller schemas + `handle_*` async handlers for the three RPC methods; param structs; the `all_about_app_controller_schemas` / `all_about_app_registered_controllers` registry pair. |
| `crates/openhuman-core/src/platform/about_app/catalog_data.rs` | The `CAPABILITIES` data itself (`LazyLock<Vec<Capability>>`) plus the shared `CapabilityPrivacy` constants, concatenated from the [`catalog_conversation_intelligence.rs`](./catalog_conversation_intelligence.rs), [`catalog_workflows_automation.rs`](./catalog_workflows_automation.rs), [`catalog_auth_channels_team.rs`](./catalog_auth_channels_team.rs), and [`catalog_localai_settings_mobile.rs`](./catalog_localai_settings_mobile.rs) submodules. |
| `crates/openhuman-core/src/platform/about_app/catalog_tests.rs` | Sibling test module (`#[path]`-included by `catalog.rs`) covering catalog behavior. |

## Public surface

Re-exported from [`mod.rs`](./mod.rs):

- **Catalog reads** (`catalog`): `all_capabilities()`, `capabilities_by_category(CapabilityCategory)`, `lookup(&str)`, `search(&str)`.
- **Ops** (`ops`): `list_capabilities(Option<CapabilityCategory>) -> Outcome<Vec<Capability>>`, `lookup_capability(&str) -> Result<Outcome<Capability>, String>`, `search_capabilities(&str) -> Outcome<Vec<Capability>>`.
- **Schema registry** (`schemas`): `about_app_schemas(&str)`, `all_about_app_controller_schemas()`, `all_about_app_registered_controllers()`.
- Types: `Capability`, `CapabilityCategory`, `CapabilityPrivacy`, `CapabilityStatus`, `PrivacyDataKind`.

## RPC / controllers

Namespace `about_app`, registered into the global controller registry via `crates/openhuman-core/src/core/all.rs`:

| Method | Inputs | Output | Description |
| --- | --- | --- | --- |
| `about_app.list` | `category` (optional enum) | `capabilities: Capability[]` | List all capabilities, optionally filtered by category. |
| `about_app.lookup` | `id` (string, required) | `capability: Capability` | Look up one capability by stable id (e.g. `local_ai.configure_provider`); errors on unknown id. |
| `about_app.search` | `query` (string, required) | `capabilities: Capability[]` | Keyword search; empty query returns all. |

Handlers deserialize params, log at `debug`, and emit `Outcome` via `into_cli_compatible_json()`. The `category` input is schema-typed as an `Option<Enum>` of all `CapabilityCategory` wire names.

## Agent tools

None. This module owns no `tools.rs`.

## Events

None. No `bus.rs`; the module neither publishes nor subscribes to `DomainEvent`s.

## Persistence

None. No `store.rs`. The catalog is a compile-time `&'static [Capability]` constant; the only runtime state is a `OnceLock<()>` (`VALIDATED`) that runs the duplicate/empty-id integrity check once.

## Dependencies

- `crate::core::Outcome`: return-type contract for ops/handlers.
- `crate::core::all::{ControllerFuture, RegisteredController}`: controller registration types (schemas.rs).
- `crate::core::{ControllerSchema, FieldSchema, TypeSchema}`: controller schema definitions (schemas.rs).

No dependencies on other `openhuman` domains: capability metadata for other domains is hand-authored text in `catalog.rs`, not imports.

## Used by

- `crates/openhuman-core/src/core/all.rs`: registers the controllers/schemas into the global RPC/CLI registry and supplies the `about_app` namespace description.
- `crates/openhuman-core/src/memory/sources/rpc/coding_sessions.rs`: references `platform/about_app/catalog_data.rs` in a doc comment, noting `max_sessions` is an untrusted, advertised programmatic RPC input (no code dependency).

## Notes / gotchas

- **`privacy: None` means "unknown", not "safe"** (per [`types.rs`](./types.rs) doc). UI must not treat an unannotated capability as local-only.
- Adding/renaming/removing a user-facing feature requires editing `CAPABILITIES`: this is the capability catalog that CLAUDE.md's "Capability catalog" rule points at. Keep ids stable; duplicate or empty ids panic at first catalog access via `ensure_validated`.
- Privacy constants encode real third-party destinations (Hugging Face, GitHub Releases, Composio `backend.composio.dev`, SearXNG, configured embedding providers, ElevenLabs, etc.): the inline comments document why several were corrected away from the generic `DERIVED_TO_BACKEND` / `LOCAL_CREDENTIALS` defaults; mirror that diligence when adding network-touching capabilities.
- `Capability` fields are all `&'static str` / copy types, so `Capability` is `Copy` and the read APIs cheaply return owned `Vec`s by copying.
- A capability's `domain` is a free-text label and does not always equal its `category` wire name (e.g. `embeddings`, `wallet`, `local_ai`, `devices`, `desktop_companion`, `security`, `tools`, `memory`).
- `CapabilityCategory::FromStr` is lenient (case-insensitive, accepts `local-ai`/`local ai`/`localai` aliases); `as_str` emits the canonical snake_case wire name used by serde.

## Further reading

- [Parent module (`platform`)](../README.md)
- [Platform and availability](../../../../../gitbooks/features/platform.md)
- [Tauri shell architecture](../../../../../gitbooks/developing/architecture/tauri-shell.md)
- [Architecture overview](../../../../../gitbooks/developing/architecture.md)
