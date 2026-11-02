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

## Path authorization

`native_paths.rs` translates OpenHuman's configuration and scoped trusted roots
into TinySecurity's `PathPolicy`. `CheckPaths2`, `ValidatePath`, and
`ValidateParent` make the policy decisions, including protected paths, reserved
workspace state, workspace/action boundaries, and symlink-aware resolution.
The host keeps only lexical input checks and OS-specific existence,
canonicalization, and symlink checks where the surrounding API requires them.

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
| [`native_paths.rs`](./native_paths.rs) | Host path context translated to TinySecurity's path-policy contract |
| [`module_gate.rs`](./module_gate.rs) | Batched path checks and cached module answers for sync-only host traits |
| [`enforcement.rs`](./enforcement.rs) | Host tool admission, budget snapshots, and scratch-directory mechanics |

See the [security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md)
for the wider host/module boundary.
