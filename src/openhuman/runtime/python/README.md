# runtime/python

The **Python interpreter client**, plus the process-launch helper for the
long-lived Python children this core owns.

## What moved out

Interpreter discovery (candidate ordering, `--version` probing, minimum-version
matching) and the managed standalone-CPython install pipeline (release index
selection, download, digest verification, extraction, atomic install,
cross-process install locking) are all in the `tinyruntime` module now, reached
through [`modules::runtime`](../../modules/runtime.rs).

## What stayed, and why

`process.rs` launches stdio Python children — the runtime Python server, and the
stdio MCP servers. That is deliberately **not** the module's pooled execution:
those children outlive a single job, speak their own protocols, and are owned by
the subsystem that started them. The module resolves the interpreter; this core
decides what to run with it.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | Export-focused: submodule decls and `pub use` re-exports. |
| `bootstrap.rs` | The interpreter client. `PythonBootstrap` (`resolve`, `probe_installed`, `try_cached`, `spawn_stdio`), `ResolvedPython`, `PythonSource`. |
| `process.rs` | `PythonLaunchSpec` and `spawn_stdio_process`: unbuffered stdio (`-u`), piped fds, `kill_on_drop`, and the Windows no-console flag. |

## Public surface

`PythonBootstrap::new(Arc<Config>)`, `.resolve() -> Result<ResolvedPython>`,
`.probe_installed()`, `.try_cached()`,
`.spawn_stdio(&PythonLaunchSpec) -> Result<tokio::process::Child>`, plus
`ResolvedPython` (`python_bin`, `bin_dir`, `version`, `source`) and
`PythonSource`.

## Persistence

**None here.** The managed install cache belongs to the `tinyruntime` module.
This module reads `config.runtime_python` (`enabled`, `prefer_system`,
`minimum_version`, `maximum_version`, `cache_dir`, `managed_release_tag`,
`preferred_command`) only to build the requests it sends.

## Dependencies

- `crate::openhuman::modules::runtime` — the module client this delegates to.
- `crate::openhuman::config` — the settings each request carries.
- `crate::openhuman::inference::local::process_util` — the Windows no-console
  hook, shared with the other child-spawning paths.

External crates: `tinyruntime-bus`, `tokio`, `anyhow`, `tracing`. No HTTP
client, no archive crates, no `walkdir`, no `fs2` — those went with the pipeline.

## Used by

- `src/openhuman/runtime/python_server/` — resolves an interpreter, then spawns
  and supervises the long-lived model server with `spawn_stdio`.
- `src/openhuman/tools/impl/system/{python_exec,shell}.rs` — hold an
  `Arc<PythonBootstrap>`; `python_exec` calls `resolve()`, `shell` uses the
  non-blocking `try_cached()` for `PATH` injection.
- `src/openhuman/skills/runtime/ops.rs` — resolves an interpreter for
  Python-backed skills.
- `src/openhuman/agent/harness_init/registry.rs` — the Python init step uses
  `probe_installed()` to decide whether provisioning is visible work.

## Notes / gotchas

- **A request names a floor, not a version.** `runtime_python.minimum_version`
  is a lower bound because the standalone channel publishes a moving set of
  builds; the exclusive `maximum_version` is how a host stays off a newer
  series. Both are interpreted by `tinyruntime-python`, not here.
- **The local cache is not redundant with the module's.** Only this one can
  answer without awaiting, which is what lets the shell inject `PATH` without
  blocking on a bus round trip.
- **`spawn_stdio` is not pooled execution.** It exists for children that outlive
  a job. Inline Python code goes through `runtime::pool::python` instead, which
  routes to the module's warm workers.
