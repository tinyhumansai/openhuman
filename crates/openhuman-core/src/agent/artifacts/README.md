# artifacts

Metadata store and lifecycle for agent-generated files (presentations,
documents, images, video). Metadata lives in the hidden, per-account
`<workspace_dir>/artifacts/`; the files themselves are written to a visible
files folder, `~/OpenHuman/projects/Files` by default
(`config::default_files_dir`, #5505). Producer tools
([`tools/impl/presentation`](../../tools/impl/presentation/), [`tools/impl/document`](../../tools/impl/document/), `media::generation`) call
this module to reserve an artifact and its file, write their bytes, and flip the record to
`Ready` or `Failed`; the module publishes the matching bus events, persists
`meta.json` / `args.json`, and exposes `ai.*` RPC controllers plus three
agent tools for listing, fetching, deleting, and regenerating. It never
renders artifact content itself.

## Key files

| File | Role |
| --- | --- |
| [`mod.rs`](./mod.rs) | `mod` decls and re-exports: `ArtifactKind` / `ArtifactMeta` / `ArtifactStatus`, the producer API (`create_artifact`, `finalize_artifact`, `fail_artifact`, `read_artifact_bytes`), and `all_artifacts_controller_schemas` / `all_artifacts_registered_controllers`. |
| [`types.rs`](./types.rs) | `ArtifactKind` (presentation/document/image/other), `ArtifactStatus` (pending/ready/failed), `ArtifactMeta`. Enums serialize lowercase; `parse` is case-insensitive and never errors (unknown kind → `Other`, unknown status → `Pending`). `ArtifactMeta.error` and `.thread_id` are optional, `skip_serializing_if` none. |
| [`files.rs`](./files.rs) | Where the bytes live: `reserve_file` (claims `<stem>.<ext>`, then `<stem> (2).<ext>`, with `create_new` in the shared files folder), `resolve_file` + the escape guard, `resolve_ready_file` (`pub`: the one id → existing-file lookup for readers and the desktop Download command), `remove_empty_placeholder`, the "file missing" error. |
| [`migrate.rs`](./migrate.rs) | `migrate_legacy_artifacts`: the idempotent, crash-safe boot move of pre-#5505 files into the files folder (copy to `.<id>.partial` → claim a name and rename → atomic meta write → delete legacy bytes). Run per account from `config::ensure_agent_dirs`. |
| [`store.rs`](./store.rs) | All filesystem I/O over `tokio::fs`: `artifacts_root`, `create_artifact`, `finalize_artifact`, `fail_artifact`, `read_artifact_bytes` (`pub`); `save_artifact_meta`, `save_artifact_args`, `read_artifact_args`, `list_artifacts`, `get_artifact`, `delete_artifact` (`pub(crate)`); `validate_artifact_id` / `assert_within_root` sandboxing; `create_artifact_regenerating` (explicit regenerate target); `sanitize_filename_stem`. |
| [`ops.rs`](./ops.rs) | RPC business logic returning `Outcome<Value>`: `ai_list_artifacts`, `ai_get_artifact`, `ai_delete_artifact`, `ai_regenerate`. `DEFAULT_LIMIT = 50`, `MAX_LIMIT = 200`. The regenerate path that re-runs `PresentationTool` is `#[cfg(feature = "documents")]`; without the feature `ai_regenerate` returns an error. |
| [`schemas.rs`](./schemas.rs) | `ControllerSchema`s and `handle_*` fns for the four `ai.*` controllers; param helpers `read_required`, `read_optional_u64`, `read_optional_string` (whitespace-only → absent), `type_name`. |
| [`tiny_store.rs`](./tiny_store.rs) | OpenHuman's side of the versioned artifacts (`tiny-artifacts`): `versioned_store` (the records, in the configured storage backend: [`version_documents.rs`](./version_documents.rs); files from earlier builds under `<workspace>/artifacts/versioned/` are imported once), `version_files_dir` (each file version's own copy), `LegacyUiBridge` (the crate's `ArtifactObserver`: shows each artifact's newest version through the legacy row + events and the files folder), `artifacts_service`, the agent tools, and `save_operator_version` (the preview panel's autosave). |
| [`production.rs`](./production.rs) | `Production`: how producer tools (deck, document, generated media) file their output — reserve the file and the card, open the versioned record under the same id, then complete or fail it. |
| `*_tests.rs` | Sibling test files for each of the above (`#[path]`). |

## Public surface

- `ArtifactKind`, `ArtifactMeta`, `ArtifactStatus`.
- Producer API, `pub` and re-exported from `mod.rs`:
  - `create_artifact(workspace_dir, kind, title, extension) -> (ArtifactMeta, PathBuf)`: mints a UUID (`create_artifact_regenerating` reuses an explicit regenerate target), creates `<root>/<id>/`, writes a `Pending` `meta.json`, publishes `ArtifactPending`, and returns the absolute path the producer should write to (`<id>/<sanitized-title>.<ext>`).
  - `finalize_artifact(workspace_dir, id, size_bytes)`: `Pending → Ready`, publishes `ArtifactReady`; no-op if already `Ready` with the same size.
  - `fail_artifact(workspace_dir, id, reason)`: `→ Failed`, stores `meta.error`, publishes `ArtifactFailed`. Logs only `reason.len()`, not the reason.
  - `read_artifact_bytes(workspace_dir, id)`: the one sanctioned id → bytes path; refuses non-`Ready` records.
- `store::save_artifact_args` is `pub(crate)` and called directly by the producers right after `create_artifact` (best-effort; failure only forfeits regeneration).
- `all_artifacts_controller_schemas` / `all_artifacts_registered_controllers`.

## RPC / controllers

Namespace `ai`. Registered in [`core/all.rs`](../../core/all.rs) under `DomainGroup::Agent` via
`all_artifacts_registered_controllers()`; `all_artifacts_controller_schemas()`
is used by [`schemas_tests.rs`](./schemas_tests.rs) to check every function has a schema and a
handler. Handlers load config through `config::rpc::load_config_with_timeout()`
and trim string params.

| Method | Inputs | Output |
| --- | --- | --- |
| `ai.list_artifacts` | `offset?: u64` (default 0), `limit?: u64` (default 50, cap 200), `thread_id?: string` | `{ artifacts: ArtifactMeta[], total, offset, limit }`: `total` is the count after the `thread_id` filter |
| `ai.get_artifact` | `artifact_id: string` | flat `ArtifactMeta` fields + `absolute_path` (root joined with `meta.path`) |
| `ai.delete_artifact` | `artifact_id: string` | `{ artifact_id, deleted: true }` |
| `ai.regenerate` | `artifact_id`, `thread_id`, `client_id` (all required) | `{ artifact_id, regenerated: true, is_error }` |

`ai.regenerate` only accepts `kind == presentation`, reloads `args.json`,
and re-runs `PresentationTool::with_regenerate_target(id)` with the
`{thread_id, client_id}` chat context passed to the tool, so the same id
is reused and the Pending/Ready/Failed events route back to the originating
chat. The RPC result is an ack; the card state comes from the socket events.
When `thread_id` is given to `list_artifacts`, legacy records with no
`thread_id` are excluded.

## Agent tools

The agent's artifact tools are the `tiny-artifacts` crate's, bound to this
workspace by [`tiny_store::registered_tools`](./tiny_store.rs) and constructed in
[`tools/ops.rs`](../../tools/ops.rs). Every write goes through the crate's
`Artifacts` service, so a tool-made artifact is a versioned record shown
through `LegacyUiBridge` exactly like a producer-made one.

| Tool | Permission | Behavior |
| --- | --- | --- |
| `create_artifact` | `Write` | New artifact with its first version: `body` for `text` / `markdown` / `html`, `path` (a file in the agent's working folder or the files folder) for `image` / `file`. |
| `update_artifact` | `Write` | Appends a version; an identical text body appends nothing. |
| `read_artifact` | default | One record with its full history. |
| `list_artifacts` | default | Every versioned record, newest first. |
| `delete_artifact` | `Dangerous` | Deletes the record, its card and file. Default-OFF: its `ToolFamily` in [`tools/user_filter.rs`](../../tools/user_filter.rs) keeps the old toggle id `artifact_delete` so saved choices still apply. |

All five classify as `DomainGroup::Agent` (`tools/ops_tool_groups.rs`), so a
harness-scoped agent keeps them. There is no regenerate tool; regeneration is
RPC-only.

## Events

Published by `store.rs` on `crate::core::bus::BUS` (no `bus.rs`; this module
subscribes to nothing):

- `DomainEvent::ArtifactPending`: from `create_artifact`.
- `DomainEvent::ArtifactReady`: from `finalize_artifact` on a real transition.
- `DomainEvent::ArtifactFailed`: from `fail_artifact`.

All three carry `thread_id` / `client_id` taken from the
`ApprovalChatContext` the producing tool receives with its run context; outside a chat turn
(CLI, cron, sub-agents) both are `None` and the
`web_chat::artifact_surface` subscriber ([`web_chat/event_bus.rs`](../../web_chat/event_bus.rs)) drops the
event.

## Persistence

- Metadata root `<workspace_dir>/artifacts/`, created on demand by
  `artifacts_root`. Per account. Not in `WORKSPACE_INTERNAL_DIRS`: the
  agent reads its large tool outputs back from `artifacts/tool-results/`,
  so a record's `file` is bounded by the escape guard instead.
- With a storage backend configured (`crate::storage`) the records (`meta.json`, `args.json`) are `artifacts` / `artifact_args` documents under the acting agent's scope instead ([`store_documents.rs`](./store_documents.rs)); the first use imports the legacy files, and the content files stay files in the files folder on every backend.
- `<root>/<id>/meta.json`: pretty-printed `ArtifactMeta`, written
  atomically (temp + rename).
- `<root>/<id>/args.json`: verbatim producer-tool args, written by the
  producer via `save_artifact_args`, read by `ai_regenerate`. Absent for
  artifacts created before it existed, which makes them non-regenerable.
- `<files_dir>/<stem>.<ext>`: the artifact bytes. `meta.file` is the
  absolute path, `meta.file_root` the files folder it was placed in, and
  `meta.path` the file name. The files folder is shared by every account on
  the OS user; `create_new` keeps names from colliding, and each account
  lists only its own metadata. Changing the folder affects new artifacts
  only; existing ones keep resolving through their recorded `file`.
- Legacy records (`file` absent) keep `meta.path` = `<id>/<filename>` under
  the metadata root until `migrate_legacy_artifacts` moves them.
- A failed generation removes its zero-byte placeholder; deleting an
  artifact removes its file and its metadata directory; a `Ready` record
  whose file was moved or deleted outside OpenHuman reads as "file missing".
- `list_artifacts` scans root subdirectories, sorts by `created_at`
  descending, applies the `thread_id` filter before pagination, and skips
  entries whose `meta.json` is missing or corrupt (`warn!`, not an error).
- Regeneration reuses the existing directory and the file it owns, and
  preserves the original `created_at` so the artifact keeps its position in
  the sorted list.

## Used by

- [`tools/impl/presentation/mod.rs`](../../tools/impl/presentation/mod.rs) and [`tools/impl/document/mod.rs`](../../tools/impl/document/mod.rs)
  (`documents` feature; in `scripts/ci/product-features.txt`, not in Cargo
  defaults), and [`media/generation/artifact_tool.rs`](../../media/generation/artifact_tool.rs): the producers.
  Presentation also uses `read_artifact_bytes` for its image pipeline.
- [`config/ops/agent.rs`](../../config/ops/agent.rs) (`ensure_agent_dirs`): creates the files folder and
  runs the migration at boot.
- [`crates/openhuman-app/src/artifact_commands.rs`](../../../../openhuman-app/src/artifact_commands.rs): the desktop Download
  command resolves an artifact id through `resolve_ready_file`.
- `web_chat/event_bus.rs`: bridges the three events to web-channel
  `artifact_*` events.
- `core/all.rs`: controller registry.

## Notes / gotchas

- Path-traversal hardening is layered: `validate_artifact_id` rejects
  empty, `.`, `..`, anything containing `/` or `\`, absolute paths, and
  `X:` drive-letter prefixes; `assert_within_root` re-checks every resolved
  path; every reader resolves through `files::resolve_file`, which refuses
  a legacy `meta.path` with `..` components (a lexical `starts_with` alone
  let `<root>/../x` through) and holds a recorded `file` to the escape guard:
  absolute, no `..`, strictly inside its `file_root` (symlinks resolved), and
  never `SecurityPolicy::is_always_forbidden`.
- `create_artifact` rejects empty titles/extensions and extensions
  containing `/`, `\`, or `.`. Filename stems are lowercased ASCII
  `[a-z0-9_-]`, capped at 80 chars, fallback `artifact`.
- Log prefix is `[artifacts]` (`[tiny-artifacts]` in `tiny_store.rs`).

## Further reading

- [Parent module README](../README.md)
- [Agent harness architecture](../../../../../docs/gitbooks/en/developing/architecture/agent-harness.md)
- [Chat](../../../../../docs/gitbooks/en/features/chat.md)
