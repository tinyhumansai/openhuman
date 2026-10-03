# Sandbox

Per-session sandbox backend selection and routed execution for agent tool
commands. This separates three concerns that used to be conflated: where a
tool runs (this module), which tools are allowed (security and tool policy),
and whether a tool needs host access (elevated ops, also owned here). The
gateway/core process itself always runs on the host; only selected tool
families (shell, filesystem, process) execute through a sandbox backend.

This domain resolves a per-session `SandboxPolicy`, owns its own Docker
backend, and delegates local OS-level confinement to `tinybox-jail`.

## Public surface

- `pub enum SandboxBackendKind { None, Local, Docker }` (`types.rs`): which
  backend a session resolved to.
- `pub struct SandboxPolicy` (`types.rs`): `backend`, `workspace_root`,
  `state_dir`, `read_only_mounts`, `read_write_mounts`, `allow_network`, `env_passthrough`,
  `docker_overrides`.
- `pub struct DockerOverrides` (`types.rs`): per-session image, network, and
  resource-limit overrides layered on `RuntimeConfig`'s `[runtime.docker]`.
- `pub struct ElevatedOp` and `pub const ELEVATED_TOOLS` (`types.rs`): tools
  (`git_operations`, `install_tool`, `docker_management`,
  `process_management`) that always require host access and must be audited
  via `build_elevated_op` rather than silently bypassing the sandbox. No code
  outside this domain calls `is_elevated_op`/`build_elevated_op` yet.
- `pub const SANDBOX_ENV_PASSTHROUGH` (`ops.rs`): the allowlisted environment
  variables (`PATH`, `HOME`, `TERM`, and so on) forwarded into sandboxed
  execution; no other host env leaks in.
- `pub fn resolve_sandbox_policy(mode: SandboxMode, action_dir, state_dir, runtime_config, is_remote_session) -> SandboxPolicy`
  (`ops.rs`): `SandboxMode::None`/`ReadOnly` resolve to
  `SandboxBackendKind::None`. `Sandboxed` resolves to `Docker` when
  `runtime_config.kind == "docker"` or the session is remote (channel/cron),
  otherwise `Local` (OS jail via `cwd_jail`). `allow_network` is
  `!is_remote_session` for `Sandboxed` and `true` otherwise; `workspace_root`
  is `action_dir`; `state_dir` is the core's internal `workspace_dir`;
  `read_only_mounts`/`read_write_mounts` carry the local jail's grant set
  (`grants.rs`, empty for other backends); `docker_overrides` is
  populated from `[runtime.docker]` only for the `Docker` backend.
- `pub async fn create_sandbox_backend(policy) -> SandboxBackendHandle`
  (`ops.rs`): instantiates and probes the resolved backend.
- `pub async fn execute_in_sandbox(policy, command, working_dir, extra_env, timeout) -> anyhow::Result<SandboxExecResult>`
  (`ops.rs`): routes to unsandboxed execution, the `cwd_jail`-based local
  jail, or Docker execution based on `policy.backend`.
- `pub fn is_elevated_op(tool_name) -> bool` and
  `pub fn build_elevated_op(...) -> ElevatedOp` (`ops.rs`): the escape hatch a
  sandboxed tool call uses to run on the host, with an audited reason.
- `pub mod docker`: Docker-specific container execution (`docker_exec`,
  `docker_backend_handle`, orphan cleanup).
- `pub use tinybox_jail as cwd_jail`: the OS-level path-confinement backend
  used by `Local`, owned by `vendor/tinybox/crates/tinybox-jail` (see its
  README). This module keeps only the policy of when to jail.
- `sandbox` RPC namespace (`schemas.rs`): `status`, `resolve_policy`,
  `cleanup_orphans`, `validate_policy`, wired through
  `all_sandbox_registered_controllers()` in
  `crates/openhuman-core/src/core/all.rs`.

## Backend behavior

- None: `execute_unsandboxed` runs the command directly via
  `agent::platform_shell` after validating `working_dir` with
  `config::ensure_usable_cwd`.
- Local: `execute_local_jail` builds a `cwd_jail::Jail` rooted at
  `policy.workspace_root`, applies `deny_net()` and any read-only mounts from
  the policy, and spawns through `cwd_jail::default_backend()` (falling back
  to `NoopBackend` if no OS jail is available on the host). Output is
  captured by redirecting stdout/stderr to files, because some backends
  (macOS Seatbelt) rebuild the command and drop piped stdio. The files live
  in a fresh per-call directory, `sandbox_capture_root(state_dir)/<uuid>/`
  (`<workspace_dir>/artifacts/sandbox-capture/<uuid>/{stdout,stderr}`), which
  the jail is granted read/write for that spawn only
  (`Jail::add_read_write`) and which is removed once read, on every exit
  path. Nothing is written into the user's project, and concurrent calls
  never share a file (#6961).
- Local grants: a real jail (Landlock, Seatbelt) denies everything it is not
  told about, so `resolve_sandbox_policy` adds the grant set from
  `grants::resolve_local_jail_grants` and `[runtime.local_jail]`
  (`LocalJailConfig`): `~/.cargo` read-write; `~/.rustup`, `~/.nvm`, `~/.npm`,
  `/usr/local`, `/opt` and the user's git config files (symlink and `include`
  targets canonicalized) read-only, each only when it exists; plus the
  `extra_read_only` / `extra_read_write` lists. When `~/.cargo` holds
  `credentials.toml` only its `bin`, `registry`, `git` and config files are
  granted. Credential stores (`~/.ssh`, `~/.gnupg`, `~/.aws`, ... and any
  parent that would contain one) are never granted, and `/proc` only with
  `allow_proc = true` (off by default: `/proc/<pid>/environ` exposes every
  process's environment). Each call also gets a private writable scratch
  directory, `sandbox_scratch_root(state_dir)/<uuid>/`, exported as `TMPDIR`
  (so `mktemp` works without granting `/tmp`) and removed afterwards.
  `create_sandbox_backend` reports `Inactive` for both the `noop` and the
  `unsupported` backend names: neither confines anything.
- Docker: `docker::docker_exec` maps the policy onto `tinybox_docker::OneShot`, which runs `docker run --rm` with the host
  `action_dir` mounted read/write at `/workspace`, network `none` by default,
  `--cap-drop ALL` (plus policy-specified extra drops),
  `--security-opt no-new-privileges`, a read-only rootfs with `/tmp` and
  `/var/tmp` tmpfs mounts, memory/CPU limits (512 MB / 1 CPU defaults), and
  only the explicit env passthrough list injected. Containers carry
  `openhuman.sandbox=true` for orphan cleanup. `validate_docker_policy`
  rejects host networking and `/`, `/etc`, `/proc`, `/sys`, or the Docker
  socket as mount roots.

## Security invariant

Sandbox backend selection is a defense-in-depth layer, not a replacement for
policy. The Rust path checks in `security` (`is_workspace_internal_path`,
`is_always_forbidden`, `classify_command`) still apply even when
`resolve_sandbox_policy` falls back to `SandboxBackendKind::None`, whether
explicitly (`SandboxMode::None`/`ReadOnly`) or implicitly, when the `Local`
backend has no usable OS jail: `cwd_jail::default_backend()` substitutes
`NoopBackend`, `execute_local_jail` in `ops.rs` spawns through it, and
`create_sandbox_backend` reports the handle as `SandboxStatus::Inactive`
rather than `Ready` so callers can tell the difference. Do not treat a
resolved `SandboxBackendKind` as a security boundary on its own.

## Dependencies

- `crate::agent::harness::definition::SandboxMode`: the agent-declared mode
  this domain resolves against.
- `crate::agent::platform_shell`: Windows-aware shell/command building for
  the unsandboxed and local-jail paths.
- `crate::config::RuntimeConfig`: `[runtime] kind`, `[runtime.docker]` and
  `[runtime.local_jail]` overrides feed `resolve_sandbox_policy`.

## Called by

- `crates/openhuman-core/src/tools/impl/system/{shell,node_exec,npm_exec,python_exec}.rs`:
  resolve a policy and call `execute_in_sandbox` per invocation.
- `crates/openhuman-core/src/flows/tinyflows/caps/code.rs`: sandboxed code
  execution capability.
- `crates/openhuman-core/src/agent/platform_shell.rs`: doc references only;
  it is the shared Windows-aware command builder that
  `execute_unsandboxed`/`execute_local_jail` call into.
- `crates/openhuman-core/src/config/ops/sandbox.rs`: RPC settings surface
  (`get_sandbox_settings`) reads `SANDBOX_ENV_PASSTHROUGH` and
  `[security.sandbox]` config.

## Tests

- `types_tests.rs`, `ops_tests.rs`, `schemas_tests.rs`, `docker_tests.rs`:
  sibling `#[cfg(test)]` suites per file.
- `ops_tests.rs` includes a regression test (#3235) pinning
  `local_status_for_backend` so a host with no OS jail reports `Inactive`
  rather than a false `Ready`.
