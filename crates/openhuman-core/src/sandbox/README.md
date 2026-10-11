# sandbox

Decides where an agent's command runs: directly on the host, inside an OS jail
confined to the action dir, or in a throwaway Docker container. The tools that
spawn processes (`shell`) and the
flows code-runner capability call into it when the agent runs in sandboxed
mode. The core process itself always runs on the host; only the spawned
command is confined.
Cron shell jobs use TinyBox's supervised passthrough process lifecycle after
host authorization; they retain host execution semantics rather than entering
the Local jail.

[`mod.rs`](./mod.rs) names three separate questions. Where a tool runs is this folder.
Which tools are allowed is the security and tool policy
(`security/`, `tools/agent_policy/`). Whether a tool needs host access at all
is the elevated-op escape hatch, which is also declared here.

## How it works

A call goes through three steps: resolve a policy, optionally probe the
backend, execute.

```text
 agent SandboxMode        action_dir, workspace_dir     RuntimeConfig
 (None|ReadOnly|Sandboxed)          |                   ([runtime] kind,
         |                          |                    [runtime.docker],
         +------------+-------------+                    [runtime.local_jail])
                      v                                       |
             resolve_sandbox_policy  <------------------------+
                      |
                      v
                SandboxPolicy { backend, workspace_root, state_dir,
                                mounts, allow_network, env, docker }
                      |
                      v
             execute_in_sandbox(policy, command, cwd, env, timeout)
                      |
       +--------------+------------------+
       v              v                  v
  None           Local                 Docker
  host shell     TinyBox StartJailedExec TinyBox RunDocker
  (env cleared)  with explicit jail     action_dir mounted at /workspace
```

### Resolving a policy

`resolve_sandbox_policy` ([`ops.rs`](./ops.rs)) maps the agent's declared
`SandboxMode` (`agent::harness::definition`) to a `SandboxBackendKind`:

- `None` and `ReadOnly` resolve to `SandboxBackendKind::None`.
- `Sandboxed` resolves to `Docker` when `runtime_config.kind == "docker"` or
  the session is remote, and to `Local` otherwise.
- `Sandboxed` resolves to `None` for the whole process when the host sets
  `OPENHUMAN_SANDBOX` (`SANDBOX_OFF_ENV`) to `off`, `none`, `0`, `false` or
  `disabled`, case-insensitive. This is for hosts that already isolate the
  core (a container, a CI or benchmark image, a VM), where the outer isolation
  permits work such as package installs or `/etc` edits that the action-dir
  jail would refuse.

The rest of the policy follows from that. `workspace_root` is the action dir
and `state_dir` is the core's internal `workspace_dir`. `allow_network` is
`!is_remote_session` for `Sandboxed` and `true` for the other modes.
`env_passthrough` is `SANDBOX_ENV_PASSTHROUGH`. `docker_overrides` is filled
from `[runtime.docker]` only for the Docker backend, and the mount lists come
from [`grants.rs`](./grants.rs) only for the Local backend.

### The None backend

`execute_unsandboxed` builds the command with `agent::platform_shell` (so
Windows gets `cmd.exe /C`), clears the environment, and forwards only
`SANDBOX_ENV_PASSTHROUGH` (`PATH`, `HOME`, `TERM`, `LANG`, `LC_ALL`,
`LC_CTYPE`, `USER`, `SHELL`, `TMPDIR`) plus the Windows process-bootstrap
variables and the caller's extra env. A passthrough variable that is set but
empty is an error. The command runs under `tools::timeout::output_or_kill`.

### The Local backend

`native_jail.rs` asks the pinned TinyBox module for `JailStatus`, creates a
module-owned `passthrough` resource rooted at `policy.workspace_root`, and
starts the shell through `StartJailedExec`. The request carries only the
resolved read-only/read-write grants, network policy, and a required enforced
isolation floor. TinyBox owns the jail provider, child process, pipe capture,
and kill/reap acknowledgement; OpenHuman reads bounded output batches and
closes the resource after terminal cleanup. A per-call scratch directory under
the core state dir is granted read-write and used for `TMPDIR`, `TEMP`, and
`TMP` unless the caller supplied one. No host-local jail or unconfined fallback
runs if the module, artifact, or requested enforcement is unavailable.

A real jail (Landlock, Seatbelt) denies everything it is not told about, so
`grants::resolve_local_jail_grants` computes what everyday commands need,
controlled by `[runtime.local_jail]` (`LocalJailConfig`):

| Grant | Access | Condition |
| --- | --- | --- |
| `~/.cargo/bin`, `~/.cargo/{config.toml,config,env}` | read-only | `toolchain_homes`, when present |
| `~/.cargo/registry`, `~/.cargo/git` | read-write | `toolchain_homes`, when present |
| `~/.rustup`, `~/.nvm`, `~/.npm`, `/usr/local`, `/opt` | read-only | `toolchain_homes`, when present |
| git config files and their `include` targets (canonicalized, depth 8) | read-only | `toolchain_homes`, when present |
| `extra_read_only`, `extra_read_write` | as listed | always, subject to the floor |
| `/proc` | read-only | only with `allow_proc = true` (default off) |

The credential floor applies to every grant: any path that
`SecurityPolicy::is_always_forbidden` rejects, or that is a parent of a
credential directory (`~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.azure`, `~/.kube`),
is dropped, because Landlock grants are recursive. `/proc` is reachable only
through `allow_proc`, since `/proc/<pid>/environ` exposes every process's
environment.

### The Docker backend

`docker::docker_exec` maps the policy to TinyBox's confidential `RunDocker`
member. TinyBox runs `docker run --rm` with the host action dir mounted
read-write at `/workspace`, network `none` by default, `--cap-drop ALL` plus
any extra drops, `--security-opt no-new-privileges`, a read-only rootfs with
`/tmp` and `/var/tmp` tmpfs mounts, memory and CPU limits (512 MB and 1 CPU by
default), and only the passthrough env plus the request's env. Containers
carry the label `openhuman.sandbox=true`; TinyBox owns the bounded daemon status
probe and label-scoped orphan cleanup. `validate_docker_policy` rejects host
networking and a mount or workspace root of `/`, `/etc`, `/proc`, `/sys` or the
Docker socket.

### Backend status

`create_sandbox_backend` returns a `SandboxBackendHandle`. None is always
`Ready`. Docker is `Ready` when the TinyBox status operation reports the daemon
available and `Error` otherwise. A module or transport failure is surfaced by
the status RPC rather than probed with a host-local Docker CLI.
Local reports `Ready` only when the module says its native jail is available
and suitable for untrusted code. Missing modules and transport errors are
reported as errors; unavailable or insufficient isolation is `Inactive`, and
execution refuses to run without the required provider.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Sandbox module declarations and public host API. |
| [`types.rs`](./types.rs) | `SandboxBackendKind`, `SandboxPolicy`, `DockerOverrides`, exec request and result, handle and status, `ElevatedOp`, `ELEVATED_TOOLS`. |
| [`ops.rs`](./ops.rs) | Policy resolution, backend creation, routed execution (None and Local paths), scratch ownership, the env allowlist, the host off switch, elevated ops. |
| [`native_jail.rs`](./native_jail.rs) | TinyBox native-jail and cron passthrough request mapping, process/output lifecycle, and fail-closed status checks. |
| [`grants.rs`](./grants.rs) | The Local jail's filesystem grant set and its credential floor. |
| [`docker.rs`](./docker.rs) | Policy-to-`RunDocker` mapping, module status/cleanup calls, Docker policy validation. |
| [`schemas.rs`](./schemas.rs) | The `sandbox.*` controllers. |

## Key types and entry points

- `SandboxPolicy` ([`types.rs`](./types.rs)): the resolved per-call policy. It is
  serializable so `sandbox.resolve_policy` can return it.
- `SandboxBackendKind` (`types.rs`): `None`, `Local`, `Docker`.
- `resolve_sandbox_policy(mode, action_dir, state_dir, runtime_config,
  is_remote_session)` (`ops.rs`).
- `execute_in_sandbox(policy, command, working_dir, extra_env, timeout)`
  (`ops.rs`): validates the working directory with `config::ensure_usable_cwd`
  (the host path for None and Local, `policy.workspace_root` for Docker) and
  routes to the backend. Returns `SandboxExecResult { exit_code, stdout,
  stderr, timed_out }`.
- `create_sandbox_backend(policy)` (`ops.rs`): probe and status.
- `SANDBOX_ENV_PASSTHROUGH` and `SANDBOX_OFF_ENV` (`ops.rs`).
- `ELEVATED_TOOLS`, `is_elevated_op`, `build_elevated_op` (`types.rs`,
  `ops.rs`): `git_operations`, `install_tool`, `docker_management` and
  `process_management` always need host access, and `build_elevated_op`
  records the reason in an audited `ElevatedOp`. No code outside this folder
  calls them yet.

## RPC surface

Registered through `all_sandbox_registered_controllers()` in `core/all.rs`:

- `sandbox.status`: backend status and availability for a backend or session.
- `sandbox.resolve_policy`: resolve a policy for a sandbox mode and
  `is_remote` flag.
- `sandbox.cleanup_orphans`: kill labelled orphan containers, returns the
  count.
- `sandbox.validate_policy`: run `validate_docker_policy` on a supplied
  policy, returns `valid` and `issues`.

The settings surface (`get_sandbox_settings`, `[security.sandbox]` and
`[runtime.docker]`) is in `config/ops/sandbox.rs`, which reads
`SANDBOX_ENV_PASSTHROUGH` and `docker::is_docker_available` from here.

## Boundaries

- The native jail and Docker providers, process ownership, and cleanup belong
  to TinyBox and run through its loadable module. This folder owns the product
  policy, approved grants, environment allowlist, and fail-closed module calls.
- Permission to run a command at all, the approval gate and the path checks
  are `security/`. The autonomy policy is off by default there, so for a
  default install the sandbox and the `is_always_forbidden` floor are what
  bound an agent.
- Shell construction for each platform is `agent::platform_shell`.
- The agent's `SandboxMode` is declared in its definition
  (`agent::harness::definition`) and read per call through
  the explicit agent sandbox mode supplied by the tool execution context.

## Gotchas

- A resolved `SandboxBackendKind` is not a security boundary on its own. The
  `security` path checks still apply when the backend is None by mode or
  `OPENHUMAN_SANDBOX=off`. Local never degrades to host execution when TinyBox
  cannot provide the requested jail; check the handle status and surface the
  refusal.
- `ShellTool::run_sandboxed` (`tools/impl/system/shell.rs`) passes
  `RuntimeConfig::default()` rather than the loaded config, while the flows
  code runner passes the loaded `runtime` block. So `[runtime] kind = "docker"` and
  `[runtime.local_jail]` do not reach the shell tool's sandbox today.
- Every current caller passes `is_remote_session = false`, so in practice
  Docker is chosen only by `[runtime] kind = "docker"`.
- The Local backend's scratch directory lives under `workspace_dir`, which
  `security` treats as internal state. It exists only for one call; output is
  captured and bounded by TinyBox.

## Tests

Sibling suites: [`types_tests.rs`](./types_tests.rs), [`ops_tests.rs`](./ops_tests.rs), [`grants_tests.rs`](./grants_tests.rs),
[`schemas_tests.rs`](./schemas_tests.rs), [`docker_tests.rs`](./docker_tests.rs) and [`docker_exec_tests.rs`](./docker_exec_tests.rs).
`native_jail_tests.rs` pins the status floor, grant/environment mapping,
module-unavailable refusal, and an ignored pinned-artifact Linux execution
fixture. Run with
`cargo test -p openhuman sandbox::` or `pnpm debug rust sandbox`.

## Further reading

- [Privacy and security](../../../../gitbooks/features/privacy-and-security.md)
- [Security architecture](../../../../gitbooks/developing/architecture/security.md)
- [tinybox submodule](../../../../vendor/tinybox/README.md)
