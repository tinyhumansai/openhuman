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
 action_dir, workspace_dir     Config ([sandbox] backend,
         |                      [runtime] kind, [runtime.docker],
         |                      [runtime.local_jail])
         +-----------+--------------+
                     v
               spawn_policy (policy.rs)
                     |
                     v
          SandboxPolicy { backend = the executor the config names, or none,
                          workspace_root, state_dir, mounts, allow_network,
                          env, docker, mount_workspace }
                     |
                     v
        execute_in_sandbox -> spawn_plan (facts, Plan2, ConsumeSandboxPlan2)
                     |
          the module's plan names the executor
       +-------------+------------------+
       v             v                  v
  Host run        Jail                Docker
  (operator       StartNativeShell-   RunDocker or
  switch, plan    Exec with the       RunDockerWithoutWorkspace
  `host_run`)     planned jail
```

### Building the request

`spawn_policy` ([`policy.rs`](./policy.rs)) builds the request side from the
configuration; it chooses nothing. `SandboxPolicy::backend` is the executor the
configuration names (`[sandbox] backend = "landlock"` is the jail, `"docker"`, a
remote session and `[runtime] kind = "docker"` are the container) or `None`. It travels to
the module as a required preference (`SandboxPlanOptions2`); with none, the module
picks the first verified executor that meets the spawn's isolation floor, so a host
without a usable jail but with Docker runs in a container. `workspace_root` is the
action dir, `state_dir` is the core's `workspace_dir`, `docker_overrides` always carries
`[runtime.docker]`, and `mount_workspace = false` (`[runtime.docker]` under the Docker
runtime) asks for a plan with no workspace grant. `with_configured_backend` fills in the
loaded configuration's backend when a caller named none.

### The operator's switch and the run on the host

`OPENHUMAN_SANDBOX=off`, `[sandbox] enabled = false` and `[components] sandbox = false`
are read by [`operator.rs`](./operator.rs) and declared to the module as the scope's
`environment` setting; the host decides nothing from them. A spawn with the switch set
first asks for a run on the host (`host_run`). The module issues that plan only for the
trusted operator source outside SaaS; it refuses (`floor`) hooks, scheduled jobs, registry
MCP servers and every SaaS scope, and the spawn is then planned onto a real executor.
`BoxPlan::from_artifact` maps the plan to `PlannedExecutor::Host`, and
`run_on_host` maps the command onto TinyBox's `StartNativeShellExec`
through [`shell_exec.rs`](./shell_exec.rs): a module-owned `passthrough` resource
rooted at the working directory, the host's own network, no jail. TinyBox
selects the shell (`cmd.exe` on Windows, bash with `pipefail` or `sh` elsewhere),
starts the child from its safe environment snapshot (`SANDBOX_ENV_PASSTHROUGH`
plus the Windows process-bootstrap variables, an empty value being an error),
applies the caller's UTF-8 overrides, owns the per-call scratch directory and
enforces the runtime budget. The host never spawns the shell itself, and without
the `modules` feature the run is refused. The host-run plan is audited and single use.

### The Local backend

`native_jail.rs` asks the pinned TinyBox module for `JailStatus`, creates a
module-owned `passthrough` resource rooted at `policy.workspace_root`, and
starts the shell through `StartNativeShellExec` with a jail. The jail is the
module's plan, not a host derivation: `ops::admit_planned` plans and reserves the
spawn and hands `BoxPlan.jail` (grants, `allow_net`, subprocess rule, enforcement,
`deny_read`, `require_process_isolation`) to `native_jail::execute` unchanged.
The host no longer picks a posture or rebuilds the request; TinyBox refuses an
executor that cannot meet the plan's isolation requirement, and a module refusal
is the typed `IsolationFloor` (never retried). `native_jail::jail_isolation` only
feeds the status report (`sandbox.status`).

| Posture reported | Backend | Meaning |
| --- | --- | --- |
| `isolated` | Kernel isolation or stronger (Landlock + namespaces) | process isolation |
| `folder_confined` | Seatbelt (macOS), no process isolation | writes confined to the folder |

TinyBox owns the jail provider, child process, pipe capture,
and kill/reap acknowledgement; OpenHuman reads bounded output batches and
closes the resource after terminal cleanup. TinyBox owns the per-call scratch
directory under the core state dir: it grants it read-write and points `TMPDIR`,
`TEMP`, and `TMP` at it unless the caller supplied one. No host-local jail or unconfined fallback
runs if the module, artifact, or requested enforcement is unavailable.

Landlock denies everything it is not told about, and Seatbelt denies every
write it is not told about, so
TinyBox's `ResolveLocalGrants` (called through `local_grants::with_resolved_grants` at spawn time) computes what everyday commands need,
controlled by `[runtime.local_jail]` (`LocalJailConfig`):

| Grant | Access | Condition |
| --- | --- | --- |
| `~/.cargo/bin`, `~/.cargo/{config.toml,config,env}` | read-only | `toolchain_homes`, when present |
| `~/.cargo/registry`, `~/.cargo/git` | read-write | `toolchain_homes`, when present |
| `~/.rustup`, `~/.nvm`, `~/.npm`, `/usr/local`, `/opt` | read-only | `toolchain_homes`, when present |
| git config files and their `include` targets (canonicalized, depth 8) | read-only | `toolchain_homes`, when present |
| `extra_read_only`, `extra_read_write` | as listed | always, subject to the floor |
| `/proc` | read-only | only with `allow_proc = true` (default off) |

The credential floor applies to every grant: any path the
protected-path floor rejects (`security::floor::floor()`, the module's
`ProtectedFloor2`, applied in `local_grants.rs`), or that is a parent of a
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
probe and label-scoped orphan cleanup. `validate_docker_policy` asks TinyBox's
`ValidateDockerPolicy` member (the check `RunDocker` itself enforces), which
reports host networking and a mount or workspace root of `/`, `/etc`, `/proc`,
`/sys` or the Docker socket; with no module there is no verdict, never a clean one.

### Backend status

`create_sandbox_backend` returns a `SandboxBackendHandle`. None is always
`Ready`. Docker is `Ready` when the TinyBox status operation reports the daemon
available and `Error` otherwise. A module or transport failure is surfaced by
the status RPC rather than probed with a host-local Docker CLI.
Local reports `Ready` when `jail_isolation` accepts the module's native jail,
and its handle's `isolation` says which posture (`isolated` or
`folder_confined`); the field is omitted for every other handle. Missing modules and transport errors are
reported as errors; unavailable or insufficient isolation is `Inactive`, and
execution refuses to run without the required provider.

### Untrusted spawns (issue #7328, phase 5)

Text the host did not author reaches a shell through exactly one gate,
`untrusted::run`: it builds the request, has the module plan it onto an executor that
is suitable for untrusted code (a refusal surfaces as `IsolationFloor`), and runs the
command in the native jail or the Docker container. There is no unjailed path: the
operator's switch (`OPENHUMAN_SANDBOX=off`, `[sandbox] enabled = false`,
`[components] sandbox = false`) is declared to the module, which never plans a run on
the host for untrusted text or under SaaS. Scheduled shell
jobs use it today. `scripts/ci/spawner-inventory.json` (checked by
`node scripts/ci/check-spawner-inventory.mjs`, part of `pnpm rust:layout`) lists
every process spawn site with its source trust; the rows still `direct` are the
open backlog and each carries the reason.

`[sandbox] backend` accepts `auto`, `landlock`, `docker` and `none`. Earlier
builds also accepted `firejail` and `bubblewrap` and `firejail_args`, but no
execution path ever existed for them, so selecting one made every command fail.
The config load migrates them to `auto` (the TinyBox-selected jail) with a
warning, and deletes `firejail_args` (`config/migrations/security_policy.rs`).

Plan adapter and read floor. [`plan.rs`](./plan.rs) (`BoxPlan::from_artifact`)
translates a TinySecurity `SandboxArtifact2` onto TinyBox's vocabulary (grants,
network, resource bounds, isolation requirement) and refuses what TinyBox cannot
enforce exactly: the Noop backend, mediated egress without a host proxy, CPU-time
bounds, zero bounds, and namespace or microVM executors the host does not drive.
A folder-confined (Seatbelt) jail, whose reads are allow-by-default, gets the
credential-store floor as `deny_read` (TinyBox contract 1.20,
`local_grants::credential_store_paths`); allow-list jails already deny those paths.

### Planning through the module (issue #7328, phase 5)

Every spawn that resolved to the native jail or a container (the agent's shell
through `execute_in_sandbox`, scheduled and hook text through `untrusted::run`) is
planned by the TinySecurity module before it launches ([`spawn_plan/`](./spawn_plan/mod.rs)).
The module owns the decision: the isolation floor (mandatory under SaaS and for text
the host did not author), whether the declared executor enforces every constraint
exactly, the filesystem and network ceilings, and the resource bounds. The host owns
the facts, the concrete operation and the execution:

1. `facts.rs` derives immutable scope facts from the resolved policy, the
   configuration and the executor TinyBox verified (`executors.rs`: a constraint is
   `Enforced` only when TinyBox says so, so a best-effort jail never satisfies a hard
   requirement). A scope's id digests its facts, so a spawn with other facts gets its
   own scope instead of widening an existing one.
2. `registry.rs` registers the scope at runtime (`RegisterSandboxScope2`, contract
   1.19) on the subject's own audit stream (`security::approval::native::audit_stream`,
   shared with the subject's approvals so their events extend one ordered ledger). The
   host keeps at most 256 scopes registered and retires the least recently used one;
   the module never evicts.
3. `plan` sends `Plan2` for the concrete operation (`operation.rs`: the command,
   working directory, environment and stdin, bound by digest where they may hold
   secrets) and maps the artifact through `BoxPlan::from_artifact`.
4. `admit` reserves the artifact once (`ConsumeSandboxPlan2`) immediately before the
   spawn and returns the policy the spawn runs under: mounts, network and container
   bounds come from the artifact, not from the policy that was asked.

A module fault, a denial, an unusable answer and a plan the host cannot map all mean
the command does not run; nothing falls back to a host-side decision. Plan binds the
exact operation, so planning happens at the spawn boundary, where the command is
known, not when the policy is built: `spawn_policy` and `command_requires_sandbox`
stay the request side (what the agent or the operator asked to sandbox and which
executor the configuration names), and the module's plan bounds what is actually run
and which executor runs it.

Network and Seatbelt, both declared to the module rather than decided here:

- **Host network.** The scope facts carry `operator_host_network` (the policy allowed
  the network), so the module plans the host's own network for that scope without a
  per-command human approval, never above the ceiling and never under SaaS. A spawn the
  policy does not allow the network for stays offline. Where the module still wants an
  approval (SaaS), the spawn is planned again with no network (it only narrows) and a
  `warn` is logged. Untrusted sources (`sandbox::untrusted`) follow the same policy
  flag, as their unjailed host-network runs did before the module; MCP stdio servers
  are jailed by TinyMCP and do not pass through this gate.
- **Folder-confined jails (macOS Seatbelt)** report best-effort enforcement. The plan
  request sets `accept_process_isolation` for the agent's own tools
  (`SpawnSource::accepts_process_isolation`), and the module then accepts a process-level
  jail for them (the artifact's `minimum_isolation` is `process`, mapped to `Declared`
  enforcement). Untrusted sources never set it, so registry MCP servers and hooks stay
  refused on macOS.

Spawn sites that do not reach the gate today, and why:

- tinymcp stdio servers start inside the loaded tinymcp module, so they do not go
  through this gate; they have their own route, below.
- tinyflows' `ProcessCodeRunner`/`ProcessShellRunner` are behind its `host-caps`
  feature, which the host does not enable; the `code` node runs through
  `execute_in_sandbox`.
- `install_tool` is currently disabled after its access and approval gates because
  package-manager scripts are not confined to an approved isolated environment; it
  does not spawn a host package manager.
- Browser and tinycomputer helper processes live in the tinycomputer module
  (their own process and OS permission model); the host spawns none of them.

## Stdio MCP servers

A stdio MCP server is third-party code that the loaded tinymcp module starts. The
host does not spawn it, so it never reaches `execute_in_sandbox`; the jail is
applied through the TinyBox contract instead (`StartStdio`, `WriteStdio`,
`ReadStdio`, `CloseStdin`, `KillStdio` and `StdioStatus`, tinybox-bus 1.21),
which tinymcp's `JailedSpawner` drives over the bus (`StdioSpawner` seam).

[`mcp/host.rs`](../mcp/host.rs) decides, per server, which way it starts:

| Server | Started |
| --- | --- |
| Declared in the operator's own `[mcp_client.servers]` | As an ordinary child process: `client_config` sets `operator_trusted`. |
| Installed from the registry (catalog installs, setup test connections) | In the TinyBox native jail. |
| Any server under SaaS (`is_saas()`), including `[mcp_client.servers]` | In the TinyBox native jail; no tenant server is operator-trusted. |

The jail is [`mcp/stdio_sandbox.rs`](../mcp/stdio_sandbox.rs): a workspace of its
own (`<workspace_dir>/mcp/stdio`), read grants from the system baseline plus
`[runtime.local_jail]` (`extra_read_only`, `extra_read_write`, and the toolchain
homes TinyBox resolves at boot with `ResolveLocalGrants`, filtered through the
same host floor as the shell jail), the credential stores in `deny_read`, network
and child processes allowed, and `require_process_isolation` on. The launcher
always asks for `Enforced`, so a backend that cannot enforce the grants, or does
not meet the untrusted-code floor (macOS Seatbelt, no kernel support), refuses and
the server does not start. tinymcp has no unconfined fallback either: a server
that is neither `operator_trusted` nor given a sandbox is refused. With the
`mcp` feature off there is no sandbox, so only the operator's servers could start,
and there is no MCP client to start them.

`scripts/ci/spawner-inventory.json` records both routes under `modules`, since
the spawn sites live in the module rather than in this repository's sources.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Sandbox module declarations and public host API. |
| [`types.rs`](./types.rs) | `SandboxBackendKind`, `SandboxPolicy`, `DockerOverrides`, exec request and result, handle and status, `ElevatedOp`, `ELEVATED_TOOLS`. |
| [`ops.rs`](./ops.rs) | Policy resolution, backend creation, routed execution (None and Local paths), scratch ownership, the env allowlist, the host off switch, elevated ops. |
| [`native_jail.rs`](./native_jail.rs) | TinyBox native-jail and cron passthrough request mapping, process/output lifecycle, and fail-closed status checks. |
| [`local_grants.rs`](./local_grants.rs) | Host adapter for `ResolveLocalGrants`: sends the credential floor as input and re-applies it to the module's reply. |
| [`docker.rs`](./docker.rs) | Policy-to-`RunDocker` mapping, module status/cleanup calls, Docker policy validation. |
| [`schemas.rs`](./schemas.rs) | The `sandbox.*` controllers. |
| [`spawn_plan/`](./spawn_plan/mod.rs) | Module planning of each sandboxed spawn: scope facts, executor declarations, operation binding, scope registry, plan and admit. |
| [`plan.rs`](./plan.rs) | `BoxPlan::from_artifact`, the artifact-to-TinyBox mapping. |

## Key types and entry points

- `SandboxPolicy` ([`types.rs`](./types.rs)): the resolved per-call policy. It is
  serializable so `sandbox.resolve_policy` can return it.
- `SandboxBackendKind` (`types.rs`): `None`, `Local`, `Docker`.
- `spawn_policy(action_dir, state_dir, sandbox_config, runtime_config,
  is_remote_session)` (`policy.rs`).
- `execute_in_sandbox(policy, command, working_dir, extra_env, timeout)`
  (`ops.rs`): validates the working directory with `config::ensure_usable_cwd`
  (the host path for a host run and the jail, `policy.workspace_root` for Docker) and
  runs the executor the module planned. Returns `SandboxExecResult { exit_code, stdout,
  stderr, timed_out }`.
- `create_sandbox_backend(policy)` (`ops.rs`): probe and status.
- `SANDBOX_ENV_PASSTHROUGH` (`ops.rs`) and `SANDBOX_OFF_ENV` (`operator.rs`).
- `ELEVATED_TOOLS`, `is_elevated_op`, `build_elevated_op` (`types.rs`,
  `ops.rs`): `git_operations`, `install_tool`, `docker_management` and
  `process_management` always need host access, and `build_elevated_op`
  records the reason in an audited `ElevatedOp`. No code outside this folder
  calls them yet.

## RPC surface

Registered through `all_sandbox_registered_controllers()` in `core/all.rs`:

- `sandbox.status`: backend status and availability for a backend or session.
  The payload also carries `suitable_for_untrusted_code` (true only for a
  ready container or a ready kernel-isolated jail), so a client can tell
  "ready" from "ready for untrusted code".
- `sandbox.resolve_policy`: resolve a policy for a sandbox mode and
  `is_remote` flag.
- `sandbox.cleanup_orphans`: kill labelled orphan containers, returns the
  count.
- `sandbox.validate_policy`: ask TinyBox (`ValidateDockerPolicy`) about a
  supplied policy, returns `valid` and `issues`; an error when the module is
  unavailable.

The settings surface (`get_sandbox_settings`, `[security.sandbox]` and
`[runtime.docker]`) is in `config/ops/sandbox.rs`, which reads
`SANDBOX_ENV_PASSTHROUGH` and `docker::is_docker_available` from here.

## Boundaries

- The native jail and Docker providers, process ownership, and cleanup belong
  to TinyBox and run through its loadable module. This folder owns the product
  policy, approved grants, environment allowlist, and fail-closed module calls.
- Permission to run a command at all, the approval gate and the path checks
  are `security/`. The autonomy policy is off by default there, so for a
  default install the sandbox and the module's protected-path floor are what
  bound an agent.
- Shell selection and the child environment for each platform are TinyBox's
  (`SelectShell` / `StartNativeShellExec`); `agent::host_runtime::host_shell_flavor`
  only tells the model which family that is.
- The agent's `SandboxMode` is declared in its definition
  (`tinyagents_definition::SandboxMode`) and read per call through
  the explicit agent sandbox mode supplied by the tool execution context.

## Gotchas

- A `SandboxBackendKind` is a request, not a security boundary. The
  `security` path checks still apply when the module plans a run on the host
  (`OPENHUMAN_SANDBOX=off`). Local never degrades to host execution when TinyBox
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

Sibling suites: [`types_tests.rs`](./types_tests.rs), [`ops_tests.rs`](./ops_tests.rs), [`local_grants_tests.rs`](./local_grants_tests.rs),
[`schemas_tests.rs`](./schemas_tests.rs), [`docker_tests.rs`](./docker_tests.rs) and [`docker_exec_tests.rs`](./docker_exec_tests.rs).
`native_jail_tests.rs` pins the status floor, grant/environment mapping,
module-unavailable refusal, and an ignored pinned-artifact Linux execution
fixture. Run with
`cargo test -p openhuman sandbox::` or `pnpm debug rust sandbox`.

## Further reading

- [Privacy and security](../../../../docs/gitbooks/en/features/privacy-and-security.md)
- [Security architecture](../../../../docs/gitbooks/en/developing/architecture/security.md)
- [tinybox submodule](../../../../vendor/tinybox/README.md)
