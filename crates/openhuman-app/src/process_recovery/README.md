# process_recovery

This directory holds only test files. The module itself is
[`src/process_recovery.rs`](../process_recovery.rs), which defines one inline submodule per platform:
`imp` (macOS), `linux_imp` and `windows_imp`. Each submodule declares its tests
with `#[path = "..."]`, and Cargo resolves those paths relative to a directory
named after the parent module, which is this one.

## What the module does

`process_recovery.rs` finds OpenHuman processes left behind by a hard exit
(crash, force quit, interrupted update) and, where it is safe, terminates
them. It exposes two functions, re-exported from whichever platform submodule
is compiled:

- `enumerate_openhuman_processes()` returns a `Vec<ProcessInfo>` (`pid`,
  `ppid`, `argv0`, `command`). The `process_diagnostics_list_owned` Tauri
  command in `lib.rs` returns this list to the frontend's diagnostics view on
  every platform.
- `reap_stale_openhuman_processes()` sends a terminate signal to stale
  processes, waits a 500 ms grace period, then force-kills any that remain.
  It does nothing when `OPENHUMAN_CORE_REUSE_EXISTING=1` is set.

`lib.rs` calls `reap_stale_openhuman_processes` only on Windows, early in
`run()` before the Tauri builder exists.

```text
run()
  |
  | Windows only:
  |   named mutex held by another instance? -> forward deep links, exit
  |   process_recovery::reap_stale_openhuman_processes()
  v
Tauri builder, setup(), ...
```

### Per platform

| Submodule | How it enumerates | What it treats as stale |
| --- | --- | --- |
| `imp` (macOS) | `ps -ax -o pid=,ppid=,command=`, filtered to commands under the running app's own `<Name>.app/Contents` (including helper bundles under `Contents/Frameworks/`). Returns nothing when the executable is not inside a `.app`. | Every match except the current pid. |
| `linux_imp` | Reads `/proc/<pid>/cmdline` and `/proc/<pid>/stat`. | Processes whose `argv0` file name is `openhuman` or `openhuman-core`, except the current pid. |
| `windows_imp` | WMIC, falling back to a PowerShell CIM query, both with `CREATE_NO_WINDOW`. | Only a wedged GUI `OpenHuman.exe`. Never `openhuman-core.exe`, never a `core`, `mcp` or `mcp-server` CLI session, never a helper with a `--type=` flag, and never an ancestor of the current process. |

The Windows rules are the strictest because the reap runs on every launch.
During an auto-update relaunch the old app is the new app's ancestor, so
excluding ancestors stops the new process from killing the one that launched
it, and the force-kill does not use a tree walk for the same reason. CLI and
MCP sessions are excluded because they may be live user sessions (for example
an MCP client). Linux does not reap at startup because its single-instance
guard registers later, so a bare reap could kill a legitimate concurrent
launch.

## Layout

| Path | Tests for |
| --- | --- |
| [`imp/process_recovery_macos_tests.rs`](imp/process_recovery_macos_tests.rs) | `ps` parsing, bundle `argv0` extraction, self filtering, and the TERM-then-KILL sequence with a fake `ProcessKiller`. |
| [`linux_imp/process_recovery_linux_tests.rs`](linux_imp/process_recovery_linux_tests.rs) | Executable name matching and exclusion of the current process from the `/proc` scan. |
| [`windows_imp/process_recovery_windows_tests.rs`](windows_imp/process_recovery_windows_tests.rs) | WMIC parsing, subcommand and `--type=` detection, ancestor exclusion, and the WMIC/CIM fallback choice. |

## Boundaries

- The TERM and force-kill primitives live in [`src/process_kill.rs`](../process_kill.rs).
- Replacing a stale core that holds the RPC port is a separate path:
  `CoreProcessHandle::ensure_running` in [`src/core_process.rs`](../core_process.rs) probes the
  listener and takes it over.

## Tests

Each file compiles only on its own platform.

```bash
cargo test --manifest-path crates/openhuman-app/Cargo.toml process_recovery::
```

## Further reading

- [`docs/gitbooks/en/developing/architecture/tauri-shell.md`](../../../../docs/gitbooks/en/developing/architecture/tauri-shell.md): the Tauri shell.
- [`crates/openhuman-app/README.md`](../../README.md): the openhuman-app crate README.
