# runtime_python

Managed Python runtime for Python-backed integrations. This domain owns interpreter discovery and process-launch primitives so callers don't need to care whether Python came from the host or a managed standalone CPython distribution. The immediate use case is launching stdio MCP servers implemented in Python. The shipped intent is a managed CPython distribution downloaded from `astral-sh/python-build-standalone`; a system-interpreter probe is a compatibility / developer override path.

## Responsibilities

- Resolve a Python ≥ `minimum_version` (default `3.12.0`) interpreter, memoizing the first success.
- Optionally probe host `PATH` for a compatible interpreter (`prefer_system`).
- Download, SHA-256-verify, extract, and atomically install a managed standalone CPython distribution when no system Python is used/available.
- Spawn line-oriented (unbuffered) Python child processes for stdio protocols such as MCP.
- Read its behavior from `[runtime_python]` config (`RuntimePythonConfig`).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/runtime_python/mod.rs` | Export-focused module root; module docstring + `pub use` re-exports of the public surface. |
| `src/openhuman/runtime_python/bootstrap.rs` | Orchestrator. `PythonBootstrap` ties resolve → (system probe \| managed install) → memoized `ResolvedPython`; exposes `spawn_stdio`. Holds the per-install file lock, cache-root selection, and managed-install probing. |
| `src/openhuman/runtime_python/resolver.rs` | System Python discovery. Walks candidate commands / `PATH`, probes `--version` (5s timeout), parses semver, enforces the minimum-version floor. |
| `src/openhuman/runtime_python/downloader.rs` | Managed distribution fetch. Queries the GitHub releases API, selects a host-compatible `install_only` asset ≥ minimum, downloads and verifies the published SHA-256 digest. |
| `src/openhuman/runtime_python/extractor.rs` | `tar.gz` extraction (gzip via `flate2`, preserves perms) + `atomic_install` (rename-into-place with backup/rollback). |
| `src/openhuman/runtime_python/process.rs` | `PythonLaunchSpec` + `spawn_stdio_process`: builds a `tokio::process::Command` with piped stdio, `-u`, `kill_on_drop`. |
| `src/openhuman/runtime_python/bootstrap_tests.rs` | Tests for the bootstrap orchestrator (`#[path]`-included). |
| `src/openhuman/runtime_python/resolver_tests.rs` | Tests for version parsing / system detection. |
| `src/openhuman/runtime_python/downloader_tests.rs` | Tests for release-metadata parse and asset selection. |

## Public surface

Re-exported from `mod.rs`:

- `bootstrap`: `PythonBootstrap`, `PythonSource` (`System` / `Managed`), `ResolvedPython` (`python_bin`, `version`, `source`).
- `downloader`: `fetch_release_metadata`, `select_distribution`, `PythonDistribution`.
- `extractor`: `atomic_install`, `extract_distribution`.
- `process`: `PythonLaunchSpec`.
- `resolver`: `detect_system_python`, `parse_python_version`, `PythonVersion`, `SystemPython`.

Primary entry points: `PythonBootstrap::new(config)`, `.resolve() -> Result<ResolvedPython>`, `.try_cached()`, `.spawn_stdio(&PythonLaunchSpec) -> Result<tokio::process::Child>`.

## RPC / controllers

None. This domain exposes no JSON-RPC controllers, schemas, or `handle_*` functions — it is a library used by other domains, not directly addressable over RPC.

## Agent tools

None. No `tools.rs`; the module owns no agent tools.

## Events

None. No `bus.rs`; the module neither publishes nor subscribes to `DomainEvent`s.

## Persistence

No structured domain store. Side effects on disk:

- Managed CPython installs land under the cache root: `config.cache_dir` if set, else `<user cache>/openhuman/runtime-python`, else `.openhuman/runtime-python`.
- Per-install exclusive file lock at `<install_dir>.lock` (`fs2`) serializes concurrent installs.
- Staging dirs (`.stage-<pid>-<uuid>`) and the downloaded archive are removed after a successful atomic install. Existing installs are moved aside to `<dest>.old-<pid>` and restored on rename failure.

## Dependencies

- `crate::openhuman::config::schema::RuntimePythonConfig` — the only intra-crate dependency; drives `enabled`, `minimum_version`, `cache_dir`, `managed_release_tag`, `prefer_system`, `preferred_command`.

External crates: `reqwest` (HTTP), `serde` (release metadata), `sha2`/`hex` (digest verify), `flate2`/`tar` (extraction), `tokio` (async fs/process + `Mutex`), `fs2` (file lock), `walkdir` (interpreter discovery), `wait-timeout` (version probe timeout), `uuid`, `dirs`, `anyhow`, `tracing`.

## Used by

Referenced from `src/openhuman/mod.rs` (module declaration) and surfaced in the capability catalog (`src/openhuman/about_app/catalog.rs`). Config wiring lives in `src/openhuman/config/schema/{runtime_python.rs,types.rs,load.rs,mod.rs}`. No other domain currently constructs `PythonBootstrap` directly in `src/` outside this wiring — the intended consumer is Python-backed integrations such as stdio MCP servers.

## Notes / gotchas

- `resolve()` is memoized: the first successful `ResolvedPython` is cached behind a `tokio::Mutex` and returned to all later callers; `try_cached()` peeks without probing.
- When `config.enabled == false`, `resolve()` bails — callers must skip Python-backed features rather than fall back.
- Managed install is only attempted when `prefer_system` is off or the system probe finds nothing compatible; the system path returns `PythonSource::System`, managed returns `PythonSource::Managed`.
- Host-asset selection prefers `install_only_stripped` assets when available; only the platform triples enumerated in `host_asset_suffix` are supported (macOS/Linux/Windows × x86_64/aarch64) — other hosts error.
- Download verification: if release metadata lacks a `digest`, SHA-256 verification is **skipped** with a warning rather than failing.
- The version probe runs `<bin> --version` with a 5s timeout, `CREATE_NO_WINDOW` on Windows, and reads version output from stdout or (fallback) stderr.
- `spawn_stdio_process` defaults to `-u` (unbuffered) and `kill_on_drop(true)` so dropped children don't leak; stdin/stdout/stderr are all piped.
- `bootstrap.rs` defines a private `install_managed()` wrapper that is currently unused by the public path (`resolve()` calls `install_managed_from_api` directly).
