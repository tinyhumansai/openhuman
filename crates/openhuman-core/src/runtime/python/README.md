# runtime/python

The **Python interpreter client**.

## What moved out

Interpreter discovery (candidate ordering, `--version` probing, minimum-version
matching) and the managed standalone-CPython install pipeline (release index
selection, download, digest verification, extraction, atomic install,
cross-process install locking) are all in the `tinyruntime` module now, reached
through [`modules::runtime`](../../modules/runtime.rs).

## Key files

| File | Role |
| --- | --- |
| [`mod.rs`](./mod.rs) | Export-focused: submodule decls and `pub use` re-exports. |
| [`bootstrap.rs`](./bootstrap.rs) | The interpreter client. `PythonBootstrap` (`resolve`, `probe_installed`, `try_cached`), `ResolvedPython`, `PythonSource`. |

## Public surface

`PythonBootstrap::new(Arc<Config>)`, `.resolve() -> Result<ResolvedPython>`,
`.probe_installed()`, `.try_cached()`, plus
`ResolvedPython` (`python_bin`, `bin_dir`, `version`, `source`) and
`PythonSource`.

## Persistence

**None here.** The managed install cache belongs to the `tinyruntime` module.
This module reads `config.runtime_python` (`enabled`, `prefer_system`,
`minimum_version`, `maximum_version`, `cache_dir`, `managed_release_tag`,
`preferred_command`) only to build the requests it sends.

## Dependencies

- `crate::modules::runtime`: the module client this delegates to.
- `crate::config`: the settings each request carries.

External crates: `tinyruntime-bus`, `tokio`, `anyhow`, `tracing`. No HTTP
client, no archive crates, no `walkdir`, no `fs2`: those went with the pipeline.

## Used by

- `crates/openhuman-core/src/tools/impl/system/{python_exec,shell}.rs`: hold an
  `Arc<PythonBootstrap>`; `python_exec` calls `resolve()`, `shell` uses the
  non-blocking `try_cached()` for `PATH` injection.
- `crates/openhuman-core/src/skills/runtime/ops.rs`: resolves an interpreter for
  Python-backed skills.

## Notes / gotchas

- **A request names a floor, not a version.** `runtime_python.minimum_version`
  is a lower bound because the standalone channel publishes a moving set of
  builds; the exclusive `maximum_version` is how a host stays off a newer
  series. Both are interpreted by `tinyruntime-python`, not here.
- **The local cache is not redundant with the module's.** Only this one can
  answer without awaiting, which is what lets the shell inject `PATH` without
  blocking on a bus round trip.
- Inline Python code goes through `runtime::pool::python`, which routes to the
  module's warm workers.

## Further reading

- [Parent module (`runtime`)](../README.md)
- [tinyruntime submodule](../../../../../vendor/tinyruntime/README.md)
- [System and utilities tools](../../../../../gitbooks/features/native-tools/system-and-utilities.md)
- [Loadable modules](../../../../../gitbooks/developing/loadable-modules.md)
