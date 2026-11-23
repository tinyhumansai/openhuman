# Security policy

OpenHuman owns the host context for security operations: autonomy settings,
workspace and action roots, per-turn trusted roots, approval routing, and
stable error markers. TinySecurity owns policy decisions for command
classification/authorization, path authorization and the action budget.

## Command authorization

Shell authorization sends the exact requested command, flow and stage to
TinySecurity through `native_commands.rs` and `modules/security/gap.rs`. Other
tools send their declared permission level and external-effect flag through
`ClassifyHostAction2`; OpenHuman does not keep a second command vocabulary or
shell parser. The host maps the module's stable command class to the approval
surface and caches class gate answers only for synchronous tool traits that
cannot await.

`OpenHumanSecurityGate` performs the async gate before dispatch. Shell uses
TinySecurity's command check directly so read-only shell commands can remain
available. Flow nodes use the same module gate before HTTP, code, and connector
operations. A missing module answer refuses the operation.

A shell command that will run inside the kernel sandbox (the run's sandbox
mode is `Sandboxed`) is checked with `check_jailed_command`
([`jailed_commands.rs`](./jailed_commands.rs)). TinySecurity's protected-path
floor is lexical: any word naming `..`, `/etc`, `/proc`, a credential store and
so on refuses the command, so `cd ..` or `cat /etc/os-release` were refused
although the jail grants only the agent's own folders. For a jailed command
that one refusal is waived. Because the module answers the floor before the
rest of its admission, the host asks again with the path words neutralized and
the original class declared, so the tier's class gate, the hidden-execution
defence, approval and the hourly budget still apply, and a command whose path
words cannot be neutralized stays refused. The security gate
(`OpenHumanSecurityGate::with_sandboxed_shell`) and the shell tool's execution
check both use it; when the spawn plan resolves such a command to the host
after all (sandbox turned off), the shell refuses it with the floor's message
(`shell_run::unjailed_floor_refusal`). Commands outside the jail, scheduled
shell jobs and every other tool keep the floor.

## Path authorization

`native_paths.rs` translates OpenHuman's configuration and scoped trusted roots
into TinySecurity's `PathPolicy`. `CheckPaths2`, `ValidatePath`, and
`ValidateParent` make the policy decisions, including protected paths, reserved
workspace state, workspace/action boundaries, and symlink-aware resolution.
The host keeps only lexical input checks and OS-specific existence,
canonicalization, and symlink checks where the surrounding API requires them.

`workspace_scope.rs::scoped_to_workspace` scopes a per-call clone of the policy
to a run's workspace descriptor root: the root becomes the `action_dir` and a
`ReadWrite` trusted root. The grant is additive and never mutates shared state;
the protected-path floor and internal-state reservations are evaluated before
any trusted-root shortcut. `FsGate::scoped_to_workspace`
(`security/tool_gates/fs.rs`) is its production caller.

Async artifact and media operations ask TinySecurity before using a resolved
path. Sync-only compatibility traits must be preceded by an async authorization
at their host call site; their callbacks do not implement a second copy of the
path policy.

## Action budget

Async acting-tool admission uses TinySecurity `PeekBudget2` and `SpendAction2`.
The host keeps a cached module snapshot for synchronous filesystem/network
traits; there is no host-side action counter. The default autonomy flag still
controls whether OpenHuman asks the module to apply the enabled tier policy.

## Key files

| File | Purpose |
| --- | --- |
| [`types.rs`](./types.rs) | Host configuration/context types, autonomy tier, stable gate/error types, and trusted roots |
| [`native_commands.rs`](./native_commands.rs) | TinySecurity command gateway and adaptation to host approval outcomes |
| [`jailed_commands.rs`](./jailed_commands.rs) | The command check for a shell command that runs in the kernel sandbox: the protected-path floor waived, every other refusal kept |
| [`local_only_commands.rs`](./local_only_commands.rs) | With security disabled: the host's conservative classifier that refuses shell commands which may reach the network under `[privacy] mode = "local_only"` |
| [`native_paths.rs`](./native_paths.rs) | Host path context translated to TinySecurity's path-policy contract |
| [`module_gate.rs`](./module_gate.rs) | Batched path checks and cached module answers for sync-only host traits |
| [`enforcement.rs`](./enforcement.rs) | Host tool admission, budget snapshots, and scratch-directory mechanics |
| [`workspace_scope.rs`](./workspace_scope.rs) | `scoped_to_workspace`: the per-call workspace-descriptor grant (`action_dir` + `ReadWrite` trusted root) |

See the [security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md)
for the wider host/module boundary.
