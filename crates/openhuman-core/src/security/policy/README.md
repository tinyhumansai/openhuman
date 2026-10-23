# Security / Policy

`SecurityPolicy` and the autonomy/risk machinery it implements. This is where
every invariant AGENTS.md lists for the autonomy policy is asked for:
`action_dir` vs `workspace_dir`, workspace-internal path protection,
always-forbidden system/credential paths, trusted roots, and the per-hour
action budget. TinySecurity answers path, protected-floor, and command-policy
questions. This folder holds the settings that module is given and the host's
projection of its answers; it decides nothing and has no local fallback (see
`native_commands.rs`, `native_paths.rs` and `module_gate.rs`). Command classes
come from TinySecurity's bus contract; TinyBox owns no command-policy vocabulary.

## Autonomy is off by default

`SecurityPolicy::enabled` mirrors `[security.autonomy] enabled` in config and defaults
to `false`. With the policy disabled, `SecurityPolicy::from_config`
([`enforcement.rs`](./enforcement.rs)) still builds the struct and hands the
flag to the module, which then answers permissively: command classification,
the approval gate, the command allowlist, the hourly action budget,
`workspace_only`, `forbidden_paths`, and the workspace-internal boundary are
all inert. The protected-path floor is not gated on `enabled` and still applies
either way: credential stores (`~/.ssh`, `~/.gnupg`, `~/.aws`, and platform
equivalents) and system roots stay unreachable, and so do `..` traversal and
null bytes in a path. The floor is the module's (`ProtectedPath2`); the host
asks it through [`../floor.rs`](../floor.rs). Setting `[security.autonomy] enabled = true`
restores the full policy. Note the asymmetry with `Default`: a policy built
from config defaults to disabled, while a policy built with no config at all
(`SecurityPolicy::default()`) defaults to enabled; see the comment on that
impl in [`types.rs`](./types.rs).

## Responsibilities

- Build a `SecurityPolicy` from `AutonomyConfig` (`SecurityPolicy::from_config`).
- Ask TinySecurity about a shell command (`check_command`) and project
  its answer into a `GateDecision`, its `CommandClass` and the host's
  policy-blocked messages (`Refusal`). The autonomy tier table is the module's;
  every caller asks it (`command_class_gate`).
- Ask the module whether filesystem paths are reachable, in one batch per
  call (`check_paths(PathKind, &[PathBuf])`, module `CheckPaths2`), whether a
  path is workspace-internal state (`is_internal`, which fails closed), and
  whether the autonomy tier allows acting at all
  (`tier_allows_acting_blocking`, module `PeekBudget2`, read-only). Trusted
  roots are part of the scope the host registers (`native_paths.rs`).
- Ask the module to spend one action of the hourly budget
  (`spend_action(name)`, module `SpendAction2`: the autonomy tier plus the
  module's single hourly ledger). The tool gate spends once per acting call;
  `spend_action` is for callers that are not tools (MCP server dispatch,
  `mcp_ui`, the workflow `http_request` node).
- Ask the module to class a host action (`host_action_class(permission,
  external)`, module `ClassifyHostAction2`).
- Use TinyTools' stable policy markers for tool-facing refusals; the markers
  are defined by `tinytools::status`, not re-exported by this module.

## Key files

| File | Purpose |
| --- | --- |
| [`types.rs`](./types.rs) | `SecurityPolicy` fields, `AutonomyLevel`, `GateDecision`, `TrustedRoot`/`TrustedAccess`, `WORKSPACE_INTERNAL_DIRS`/`WORKSPACE_INTERNAL_FILES` |
| [`path_checks.rs`](./path_checks.rs) | `validate_path` and `validate_parent_path`: the module-backed path authorization that returns the canonical path |
| [`native_paths.rs`](./native_paths.rs) | Translation of trusted roots, the acting directory and the internal-state reservations into the module's immutable path scope |
| [`module_gate.rs`](./module_gate.rs) | `check_paths[_blocking]`, `path_allowed[_blocking]`, `is_internal[_blocking]`, `spend_action`, `tier_allows_acting_blocking`, `host_action_class`, and the `PathKind` and `Budget` types |
| [`command_checks.rs`](./command_checks.rs) | `parse_declared_class` |
| [`native_commands.rs`](./native_commands.rs) | `SecurityPolicy::check_command` / `command_class_gate[_blocking]`, `CommandQuery`, `CommandFlow`, `CommandStage`, `CommandVerdict`, `Refusal`, and the `CommandGateway` seam to TinySecurity's command policy |
| [`enforcement.rs`](./enforcement.rs) | `from_config`, `with_privacy_mode`, `openhuman_scratch_dir`/`ensure_openhuman_scratch_dir`, `validate_path_within_root` |
| TinySecurity command-policy contract (`tinysecurity-bus`) | Command class, gate, risk and refusal category for one command under the host-registered policy; `tinysecurity_bus::CommandClass` is only the host-facing compatibility vocabulary |

## Public surface

`crate::security::policy` re-exports `SecurityPolicy`, `AutonomyLevel`,
`GateDecision`, `PathKind`, `Budget`, `host_action_class`, `TrustedRoot`,
`TrustedAccess`, the command gateway/query types, and path/scratch helpers.
`CommandClass` comes from `tinysecurity_bus`, which owns command classification;
policy markers come from `tinytools::status`. `crate::security` re-exports the commonly used policy
types and path/scratch helpers, but not the command gateway/query types.

`security/live_policy.rs` (a sibling of this directory) holds the current
`SecurityPolicy` in a process-global cell: new sessions `install` the latest
policy, and `reload_from` / `reload_privacy` swap it the moment the config is
saved. `security_scoped_to_root` (`tools/impl/filesystem/gate.rs`, reached through
`FsGate::scoped_to_workspace`) and `security_for_tool_context`
(`tools/impl/system/mod.rs`) clone a tool's policy per call and, when the run
carries a workspace descriptor, set `action_dir` to that root and pushes it
as a `ReadWrite` `TrustedRoot`. The module evaluates the protected-path floor
and the internal-state reservations before any trusted-root grant, so the
grant cannot widen them.

## Invariants

Each of these must not be weakened to make a feature work:

- `action_dir` vs `workspace_dir`. `action_dir` is the agent's action
  sandbox root; tools resolve relative paths and default their cwd there.
  `workspace_dir` holds core-internal state (memory DBs, sessions, tokens);
  the workspace root itself is a permitted containment root
  in the registered scope, but its internal-state subtree is refused
  by the module's path scope before any trusted-root grant is
  consulted. Kept as two separate fields on `SecurityPolicy` (`types.rs`) so
  the two roots can diverge.
- Workspace-internal state is the module's call (`is_internal`, kind
  `Internal`, which fails closed). The host supplies the reservation list
  (`WORKSPACE_INTERNAL_DIRS`, `WORKSPACE_INTERNAL_FILES` and
  `WORKSPACE_INTERNAL_PREFIXES` in `types.rs`: `memory-`, `memory_tree-`,
  `session_raw-`) in the scope [`native_paths.rs`](./native_paths.rs)
  registers, and the module matches it against the canonical form of the
  paths.
- The protected-path floor is the module's (`ProtectedPath2`, asked through
  [`../floor.rs`](../floor.rs): `is_protected`, `any_protected`,
  `protected_paths`, `is_protected_blocking`). It matches credential-store
  components (`.ssh`, `.gnupg`, `.aws`, `.azure`, `.kube`, `keychains`,
  Windows DPAPI directories) case-insensitively by path segment, plus system
  root prefixes (`/etc`, `/root`, `/boot`, `/proc`, `/sys`, `/system`,
  `C:\Windows`, `C:\Program Files[  (x86)]`, `C:\ProgramData`), and rejects
  `..` traversal and null bytes. It is unconditional and needs no policy; a
  `trusted_root` grant can never reach these paths. A module that cannot
  answer protects the path. The vocabulary itself (`floor()`, module
  `ProtectedFloor2`) is read by `sandbox/local_grants.rs` to filter TinyBox
  jail grants. Gray-area directories (`/usr`, `/opt`, `/var`, `~/Library`)
  deliberately stay in the user-overridable `forbidden_paths` list instead, so
  a grant can still reach, for example, `/usr/local/...`.
- The command floor is the module's, and it fails closed. A command that is not
  provably read-only is at least `CommandClass::Write`; the highest class across
  `;`/`|`/`&&`/`||`/newline-separated segments wins; an unquoted redirect or
  `tee` lifts it to at least `Write`; a quoted heredoc body is data. An
  LLM-declared category (`parse_declared_class`) is escalate-only: it travels
  as `declared_class` and the module takes the maximum, so the model can raise
  the gate but never lower what the module determined. The module's answer is
  `Allow`, `Prompt` or a refusal; a module that cannot answer is
  `Refusal::Unavailable`, never an allow. Nothing in this folder classifies a
  command.
- Approval gate default-on, marker semantics. `POLICY_BLOCKED_MARKER`
  (`[policy-blocked]`) prefixes a permanent rejection: the identical call
  can never succeed in the current tier (read-only blocking a write, a
  forbidden/credential path, a disallowed high-risk or hidden-execution
  command). `POLICY_DENIED_MARKER` (`[policy-denied]`) prefixes a this-turn
  denial: the user said no to an approval prompt, or it timed out. The agent
  harness's `RepeatedToolFailureMiddleware` keys on these to halt on the first
  verbatim repeat instead of reiterating a provably futile call. See
  [`../approval/README.md`](../approval/README.md) for the gate itself and its
  10-minute default TTL.
- Hourly action budget. The module keeps one ledger (the command scope
  registered for the Harness flow at the Execute stage). The tool gate spends
  one action per acting call (`Check::HostAction { spend }`, installed by
  `ToolPolicyGateMiddleware` in `agent/tinyagents/middleware/security_gate.rs`),
  so the tools themselves no longer count. Callers that are not tools spend
  through `SecurityPolicy::spend_action`, which refuses once the count exceeds
  `max_actions_per_hour`. The tier alone (no budget) is
  `tier_allows_acting_blocking`, used by callers that write at a finer grain
  than one tool call, for example the kernel memory guard under
  `MemoryCore::store`, and by `FsGate`/`NetGate::can_act`.

## Tests

- [`policy_tests.rs`](./policy_tests.rs), [`policy_allowlist_tests.rs`](./policy_allowlist_tests.rs),
  [`policy_characterization_tests.rs`](./policy_characterization_tests.rs),
  [`policy_config_and_action_tests.rs`](./policy_config_and_action_tests.rs),
  [`policy_paths_and_risk_tests.rs`](./policy_paths_and_risk_tests.rs),
  [`policy_trusted_roots_tests.rs`](./policy_trusted_roots_tests.rs) and
  [`policy_action_dir_tests.rs`](./policy_action_dir_tests.rs): behavior tests for
  policy construction, trusted roots, the acting directory and tier handling.
- [`policy_disabled_tests.rs`](./policy_disabled_tests.rs): behavior with `[security.autonomy] enabled = false`.
- [`module_gate_tests.rs`](./module_gate_tests.rs): the projection of the module's path and
  budget answers, with a scripted `GapPort` (`modules/security/gap`) standing in for the module.
  Unit tests install a scripted `StubPort` for the current thread; there is no local policy to
  fall back on.
- `native_paths_tests.rs` and `path_checks_native_tests.rs`: the scope translation and the
  module-backed `validate_path`.
- [`native_commands_tests.rs`](./native_commands_tests.rs) and `native_commands_stub.rs`: the projection of the module's answer, the fail-closed paths, and a scripted `CommandGateway` for tests of what a caller does with each answer. What the module itself answers is pinned by the module-backed tests in `modules/security_command_tests.rs` and `modules/security_command_characterization_tests.rs`, which run against the released archive.
- [`enforcement_scratch_dir_tests_tests.rs`](./enforcement_scratch_dir_tests_tests.rs): `openhuman_scratch_dir` is
  namespaced on every platform and `ensure_openhuman_scratch_dir` creates it.

## Further reading

- [Parent module (`security`)](../README.md)
- [Security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md)
- [Privacy and security](../../../../../docs/gitbooks/en/features/privacy-and-security.md)
- [Approval gate](../../../../../docs/gitbooks/en/features/approval-gate.md)
- [OS keyring and secret storage](../../../../../docs/gitbooks/en/features/os-keyring-and-secret-storage.md)
