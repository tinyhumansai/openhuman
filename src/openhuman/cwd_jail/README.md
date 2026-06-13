# cwd_jail

Cross-platform directory-jail facade. Given a declarative description of a workspace (`Jail`), it picks the strongest available OS sandbox backend and spawns a child process caged into a single read/write root (plus optional read-only paths), with toggles for outbound network and subprocess creation. It is the user-facing complement to `src/openhuman/security/` — the autonomy gate decides *whether* a command may run; `cwd_jail` decides *what filesystem* the approved child process sees. It jails the *child* it spawns, never the core process itself.

## Responsibilities

- Describe a jail declaratively via a builder (`Jail::new(root, label)` + `.add_read_only(...)`, `.deny_net()`, `.deny_subprocess()`).
- Auto-detect and cache the strongest backend for the current OS (Landlock / Seatbelt / AppContainer / noop).
- Spawn a `std::process::Command` inside the jail, canonicalizing `root` (and read-only paths) first so backends never see `..` or symlink trickery.
- Provide a persistent registry to manage many jailed workspaces side-by-side, each with a stable id, label, directory, and metadata, indexed in a JSON file.
- Fall back gracefully (`noop`) when no OS-level sandbox is available, while still letting callers rely on application-layer path checks.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/cwd_jail/mod.rs` | Module docstring + the thin facade: `spawn` / `spawn_with` / `default_backend` (cached via `OnceLock`). Re-exports the public surface. Inline tests. |
| `src/openhuman/cwd_jail/jail.rs` | Core types: the `Jail` description struct (builder + `canonicalize`/`canonicalize_or_log`) and the `JailBackend` trait (`name`/`is_available`/`spawn`). |
| `src/openhuman/cwd_jail/detect.rs` | `pick_backend()` — cfg-gated platform selection; returns the first available backend or `NoopBackend`. |
| `src/openhuman/cwd_jail/noop.rs` | `NoopBackend` — no enforcement, plain `Command::spawn`. Always available. |
| `src/openhuman/cwd_jail/linux.rs` | `LandlockBackend` — kernel 5.13+ Landlock LSM applied in `pre_exec` (child-side, after fork, before exec). Gated on the `sandbox-landlock` cargo feature. |
| `src/openhuman/cwd_jail/macos.rs` | `SeatbeltBackend` — wraps the command in `/usr/bin/sandbox-exec -p '<profile>'`. Renders an allow-default-reads / deny-default-writes Seatbelt profile. |
| `src/openhuman/cwd_jail/windows.rs` | `AppContainerBackend` — `CreateAppContainerProfile` + DACL grant + `STARTUPINFOEX`/`CreateProcessW` via `windows-sys`. |
| `src/openhuman/cwd_jail/registry.rs` | `JailRegistry` + `JailRecord` — multi-jail manager persisted to `index.json`, with atomic-rename writes and containment checks. |
| `src/openhuman/cwd_jail/registry_tests.rs` | Sibling test suite for the registry (`#[path]`-included from `registry.rs`). |

## Public surface

Re-exported from `mod.rs`:

- `Jail`, `JailBackend` — the declarative jail description and the OS-enforcement trait.
- `NoopBackend` — the unenforced fallback backend.
- `JailRecord`, `JailRegistry` — persisted multi-jail manager.

Free functions in `mod.rs`:

- `default_backend() -> Arc<dyn JailBackend>` — process-wide cached, lazily auto-detected backend.
- `spawn(jail: &Jail, cmd: Command) -> io::Result<Child>` — canonicalize + spawn under the default backend.
- `spawn_with(backend: &dyn JailBackend, jail: &Jail, cmd: Command) -> io::Result<Child>` — same, with an explicit backend (tests / forced weaker backend).

`JailRegistry` methods: `open`, `base`, `create`, `get`, `list`, `find_by_label`, `rename`, `set_notes`, `delete`, `clear`, `spawn_in`, `spawn_in_with`.

## Persistence

`JailRegistry` is rooted at a base directory (e.g. `~/.openhuman/jails/` or `<workspace>/jails/`). Each jail is a `<base>/<id>/` directory; metadata for all jails lives in `<base>/index.json`.

- `JailRecord` fields: `id`, `label`, `dir`, `backend_at_create`, `created_at_unix`, `updated_at_unix`, optional `notes`.
- The on-disk index is the source of truth; in-memory state (`Index`, a `BTreeMap` for deterministic ordering) is rebuilt on every `open()`.
- Writes are atomic (write-temp + rename, with a direct-overwrite fallback if rename-over fails). Mutating ops roll back the in-memory change if persistence fails.
- Ids are generated as `j<ts_hex><counter_hex>` (not cryptographically random — used as directory names), with a collision-retry loop on `create`.
- Concurrency is guarded by a `std::sync::Mutex`; multi-*process* access is an explicit non-goal (no OS file locking).

## Dependencies

This module is notably self-contained: its own files contain **no** `use crate::openhuman::` or `use crate::core::` imports. Dependencies are external/std only:

- `std::process` (`Command`/`Child`), `std::fs`, `std::sync` (`Mutex`/`OnceLock`/`Arc`), `std::time`.
- `serde` / `serde_json` — `JailRecord` and the index serialization (registry).
- `landlock` crate — Linux backend, gated on the `sandbox-landlock` cargo feature.
- `windows-sys` — Windows AppContainer FFI (Security/Isolation, Threading, Memory APIs).
- macOS backend shells out to the system binary `/usr/bin/sandbox-exec`.

The Linux backend's docstring references `crate::openhuman::security::landlock` as conceptual prior art, but the implementation here is self-contained (it does not import that module).

## Used by

- Declared at `src/openhuman/mod.rs:37` (`pub mod cwd_jail;`). No other `src/` Rust files currently reference `openhuman::cwd_jail` — it is a self-standing facade not yet wired into a calling domain.

## Notes / gotchas

- **Not RPC-facing, no agent tools, no event bus.** There is no `schemas.rs`, `tools.rs`, or `bus.rs`; the module exposes no `openhuman.*` RPC methods, owns no agent tools, and publishes/subscribes to no `DomainEvent`s.
- **Backends differ in what `allow_net` / `read_only` mean.** Landlock does not gate network at all; macOS Seatbelt grants `allow default` (full network) and treats `read_only` as informational since reads are already allowed; Windows AppContainer is the only backend that honors `read_only` as a real read grant and maps `allow_net` to the strictly outbound-only `internetClient` capability (LAN/inbound capabilities deliberately excluded).
- **Windows `spawn` currently returns an error after a successful launch.** `spawn_in_container` creates the process successfully but cannot bridge the raw `HANDLE` into a `std::process::Child` (the needed `FromRawHandle for Child` is unstable), so it closes the handle and returns `io::ErrorKind::Unsupported`. See the TODO in `windows.rs`. The Windows path is compile-checked but flagged as needing real-hardware testing.
- **macOS stdio is inherited** — the Seatbelt wrapper cannot re-apply the original command's `Stdio` config; it uses `sandbox-exec` defaults (inherit). The profile re-allows writes only under canonicalized `root` + `/private/tmp`, so callers must canonicalize first (the `spawn` facade does this automatically) or writes inside `root` may be denied (e.g. `/tmp` → `/private/tmp`).
- **Linux Landlock runs in `pre_exec`** (child-side after fork) so the parent keeps its privileges; read-only paths also get `Execute` so the child can run binaries found there (e.g. `/usr/bin/sh`).
- **Registry containment guard:** both `delete` and `jail_for` (used by `spawn_in`/`spawn_in_with`) refuse to operate on a record whose canonicalized `dir` is not under the canonicalized `base`, defending against a corrupted index pointing at `/`.
- **Free-form input is length-logged, not value-logged** (labels/notes) to avoid leaking arbitrary text into logs.
