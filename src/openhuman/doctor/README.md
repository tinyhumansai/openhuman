# doctor

Diagnostic / self-check domain for OpenHuman. Runs a synchronous battery of probes against the live `Config`, the workspace directory, the daemon state file, the local environment, the memory-tree SQLite DB, the embedding provider (Ollama), and the Claude Agent SDK binary, then aggregates the findings into a severity-tagged `DoctorReport`. Exposed to CLI and JSON-RPC as `doctor.report` and `doctor.models`. This is what powers `openhuman doctor` / the Settings health surface.

## Responsibilities

- Validate config semantics: config file presence, `api_url` (with fallback resolution), app session JWT / sign-in state, `default_model`, temperature range (0.0–2.0), legacy `reliability.fallback_providers`, model & embedding route entries (empty hints/models, invalid embedding providers, `dimensions=0`), `memory.embedding_model` `hint:` cross-reference, at least one channel configured, and delegate-agent models.
- Check workspace integrity: directory existence, write probe (create/write/delete a temp probe file), `memory/` dir, `SYSTEM.md` prompt, and best-effort free disk space (warns under 512 MB; uses `df -m` on Unix, PowerShell `Get-PSDrive` on Windows).
- Inspect daemon state file: presence, JSON validity, heartbeat freshness (stale > 30s = error), scheduler component health (stale > 120s), and per-channel component freshness (stale > 300s).
- Probe environment commands: `git --version`, `curl --version`, `$SHELL`, and `$HOME`/`$USERPROFILE`.
- Probe memory-tree DB: warn on stale `-shm`/`-wal` SQLite side-files or not-yet-created DB; otherwise open the DB and run `SELECT COUNT(*) FROM mem_tree_chunks`.
- Probe embedding-model health: if provider is `ollama`, do a 3s blocking HTTP GET to `<base_url>/api/tags` and verify the configured model is installed; non-ollama providers report OK without a local probe.
- Probe the Claude Agent SDK: if enabled, run `<binary> --version`.
- Probe provider model availability via `run_models` (currently a stub — see Notes).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/doctor/mod.rs` | Module docstring + exports. Declares `core`, `ops`, `schemas`; re-exports `core::*`, `ops::*` (also aliased `pub use ops as rpc`), and the schema controller pair. |
| `src/openhuman/doctor/core.rs` | All diagnostic logic + types. `run()` entry point and every `check_*` probe; `run_models()`; severity helpers; OS-specific disk/command helpers. |
| `src/openhuman/doctor/ops.rs` | Async JSON-RPC/CLI controller surface (`doctor_report`, `doctor_models`) wrapping the sync `core` logic in `spawn_blocking` and returning `RpcOutcome<T>`. |
| `src/openhuman/doctor/schemas.rs` | Controller schemas + registry (`all_controller_schemas`, `all_registered_controllers`, `handle_report`/`handle_models`). |
| `src/openhuman/doctor/core_tests.rs` | Test suite for `core.rs` (via `#[path = "core_tests.rs"] mod tests`). |

## Public surface

From `mod.rs` re-exports (`core::*`):

- Types: `Severity` (`Ok`/`Warn`/`Error`), `DiagnosticItem`, `DoctorSummary`, `DoctorReport`, `ModelProbeOutcome`, `ModelProbeEntry`, `ModelProbeSummary`, `ModelProbeReport`.
- Functions: `run(&Config) -> Result<DoctorReport>` (blocking-only — keep no `.await` inside), `run_models(&Config, use_cache) -> Result<ModelProbeReport>`.

From `ops` (also re-exported as `doctor::rpc`):

- `doctor_report(&Config) -> Result<RpcOutcome<DoctorReport>, String>`
- `doctor_models(&Config, use_cache: bool) -> Result<RpcOutcome<ModelProbeReport>, String>`

Schema pair re-exported as `all_doctor_controller_schemas` / `all_doctor_registered_controllers`.

## RPC / controllers

Namespace `doctor`, two functions:

| Method | Inputs | Output |
| --- | --- | --- |
| `doctor.report` | none | `DoctorReport` ("Run diagnostics for workspace and runtime configuration.") |
| `doctor.models` | `use_cache: Option<bool>` (default `true`) | `ModelProbeReport` ("Probe provider model availability and auth status.") |

Both handlers load config via `config_rpc::load_config_with_timeout()` and return `RpcOutcome::single_log(...)`. Wired into the global registry in `src/core/all.rs` (controllers, schemas, and the namespace description "Run diagnostics for workspace and runtime health.").

## Agent tools

None. This domain owns no agent tools (no `tools.rs`).

## Events

None. No `bus.rs` / event-bus subscribers or publishers.

## Persistence

None of its own (no `store.rs`). It only **reads** existing state owned by other domains: the daemon state file (`service::daemon::state_file_path`), the memory-tree SQLite DB (`<workspace>/memory_tree/chunks.db`), config files, and `<workspace>/SYSTEM.md` / `<workspace>/memory/`. Its only writes are an ephemeral workspace probe file that is immediately deleted.

## Dependencies

- `crate::openhuman::config::{Config, rpc}` — reads the live config for all probes; `config_rpc::load_config_with_timeout` in the handlers.
- `crate::openhuman::service::daemon` — `state_file_path` for the daemon heartbeat/component snapshot.
- `crate::openhuman::memory_store::{chunks::store, factories}` — `with_connection` for the DB probe; `effective_embedding_settings` to resolve the intended embedding provider/model.
- `crate::openhuman::inference::{provider, local}` — `provider::list_providers` (model targets) and `local::ollama_base_url` (embedding probe).
- `crate::api::{config, jwt}` — `effective_api_url` fallback resolution and `get_session_token` for sign-in state.
- `crate::core::all::{ControllerFuture, RegisteredController}`, `crate::core::{ControllerSchema, FieldSchema, TypeSchema}` — controller/schema plumbing.
- `crate::rpc::RpcOutcome` — handler return contract.
- External: `reqwest` (blocking client + URL parse), `serde`/`serde_json`, `chrono`, `anyhow`.

## Used by

- `src/core/all.rs` — registers the doctor controllers/schemas into the global RPC + CLI registry (the only in-tree consumer).

## Notes / gotchas

- `run()` is **strictly blocking** by contract (file system, sqlite, blocking HTTP). `reqwest::blocking::Client` panics inside a tokio runtime, so `ops::doctor_report` runs the whole thing in `tokio::task::spawn_blocking`. Do not add `.await` inside `core::run`.
- `run_models` / `doctor.models` is effectively a **stub**: it enumerates providers from `inference::provider::list_providers` but marks every entry `Skipped` with message "model catalog refresh removed" (catalog refresh was removed). It never actually probes auth/availability despite the schema description.
- The embedding probe is capped at a 3s timeout to avoid stalling on a slow Ollama daemon; non-ollama providers short-circuit to OK.
- `model_matches` treats `name` vs `name:tag` as a match only when at most one side is tagged; two differently-tagged names are not considered equal.
- Severity rollup: `DoctorSummary` counts `Ok`/`Warn`/`Error` items; checks are intentionally lenient (missing optional dirs/tools → `Warn`, not `Error`).
- OS-specific helpers (`available_disk_space_mb`, `check_command_available`, `check_claude_agent_sdk`) set `CREATE_NO_WINDOW` on Windows to avoid console flashes.
