# Security / Policy

`SecurityPolicy` and the autonomy/risk machinery it implements. This is where
every invariant AGENTS.md lists for the autonomy policy is actually enforced:
`action_dir` vs `workspace_dir`, workspace-internal path protection,
fail-closed command classification, always-forbidden system/credential paths,
trusted roots, and the per-hour action budget.

## Autonomy is off by default

`SecurityPolicy::enabled` mirrors `[autonomy] enabled` in config and defaults
to `false`. With the policy disabled, `SecurityPolicy::from_config`
([`enforcement.rs`](./enforcement.rs)) still builds the struct, but `can_act()`,
`record_action()`, and `is_rate_limited()` all short-circuit: command
classification, the approval gate, the command allowlist, the hourly action
budget, `workspace_only`, `forbidden_paths`, and the workspace-internal
boundary are all inert. `is_always_forbidden` is not gated on `enabled` and
still applies either way: credential stores (`~/.ssh`, `~/.gnupg`, `~/.aws`,
and platform equivalents) and system roots stay unreachable, and so do `..`
traversal and null bytes in a path. Setting `[autonomy] enabled = true`
restores the full policy. Note the asymmetry with `Default`: a policy built
from config defaults to disabled, while a policy built with no config at all
(`SecurityPolicy::default()`) defaults to enabled; see the comment on that
impl in [`types.rs`](./types.rs).

## Responsibilities

- Build a `SecurityPolicy` from `AutonomyConfig` (`SecurityPolicy::from_config`).
- Classify shell commands into a risk level (`CommandRiskLevel`) and a
  fail-closed gate class (`CommandClass`), and turn that into a `GateDecision`
  for the current `AutonomyLevel`.
- Decide whether a filesystem path is reachable at all
  (`is_always_forbidden`), workspace-internal state
  (`is_workspace_internal_path`), or covered by a `TrustedRoot`.
- Track and enforce the hourly action budget (`ActionTracker`).
- Own the two stable string markers (`POLICY_BLOCKED_MARKER`,
  `POLICY_DENIED_MARKER`) that the agent harness's repeated-failure
  middleware keys on to stop retrying a provably futile call.

## Key files

| File | Purpose |
| --- | --- |
| [`types.rs`](./types.rs) | `SecurityPolicy` fields, `AutonomyLevel`, `CommandRiskLevel`, `CommandClass`, `GateDecision`, `ToolOperation`, `ActionTracker`, `TrustedRoot`/`TrustedAccess`, `WORKSPACE_INTERNAL_DIRS`/`WORKSPACE_INTERNAL_FILES`, the two policy markers |
| [`path_checks.rs`](./path_checks.rs) | `is_workspace_internal_path`, `is_always_forbidden`, `check_cross_profile`, `is_within_trusted_root`, `is_resolved_path_allowed[_for]`, `check_resolved_against_forbidden` |
| [`command_checks.rs`](./command_checks.rs) | `classify_command`, `gate_decision`, `check_gated_command`, `is_command_allowed`, `command_risk_level`, `parse_declared_class`, `is_command_executor`, `split_unquoted_segments` |
| [`enforcement.rs`](./enforcement.rs) | `can_act`, `enforce_write_tier`, `enforce_tool_operation`, `record_action`/`is_rate_limited`, `from_config`, `with_privacy_mode`, `openhuman_scratch_dir`/`ensure_openhuman_scratch_dir`, `validate_path_within_root` |
| TinyBox module `CommandAnalysis` (`tinybox-bus`) | Module-produced command segments, classifications, structural flags, and literal path words; the host applies autonomy, allowlist, approval, and protected-path policy to these facts |

## Public surface

Re-exported through `policy/mod.rs` and then through `security/mod.rs`:
`SecurityPolicy`, `AutonomyLevel`, `CommandClass`, `CommandRiskLevel`,
`GateDecision`, `ToolOperation`, `ActionTracker`,
`TrustedRoot`, `TrustedAccess`, `POLICY_BLOCKED_MARKER`,
`POLICY_DENIED_MARKER`, `validate_path_within_root`,
`ensure_openhuman_scratch_dir`, `openhuman_scratch_dir`.

`security/live_policy.rs` (a sibling of this directory) holds the current
`SecurityPolicy` in a process-global cell: new sessions `install` the latest
policy, and `reload_from` / `reload_privacy` swap it the moment the config is
saved. `security_scoped_to_root` (`tools/impl/filesystem/gate.rs`, reached through
`FsGate::scoped_to_workspace`) and `security_for_tool_context`
(`tools/impl/system/mod.rs`) clone a tool's policy per call and, when the run
carries a workspace descriptor, set `action_dir` to that root and pushes it
as a `ReadWrite` `TrustedRoot`. `is_always_forbidden` and
`is_workspace_internal_path` are evaluated before any trusted-root shortcut,
so the grant cannot widen them.

## Invariants

Each of these must not be weakened to make a feature work:

- `action_dir` vs `workspace_dir`. `action_dir` is the agent's action
  sandbox root; tools resolve relative paths and default their cwd there.
  `workspace_dir` holds core-internal state (memory DBs, sessions, tokens);
  the workspace root itself is a permitted containment root
  (`is_resolved_path_allowed_for`), but its internal-state subtree is refused
  by `check_resolved_against_forbidden` before any trusted-root grant is
  consulted. Kept as two separate fields on `SecurityPolicy` (`types.rs`) so
  the two roots can diverge.
- `is_workspace_internal_path` ([`path_checks.rs`](./path_checks.rs)) is true for any path
  whose first component under `workspace_dir` is in `WORKSPACE_INTERNAL_DIRS`
  or `WORKSPACE_INTERNAL_FILES` (`types.rs`), or starts
  with `memory-`, `memory_tree-`, or `session_raw-`. It is checked against the
  canonicalized form of both paths when possible, falling back to the raw
  paths for not-yet-existing targets.
- `is_always_forbidden` (`path_checks.rs`) does a case-insensitive,
  segment-based match against `SENSITIVE_COMPONENTS` (`.ssh`, `.gnupg`,
  `.aws`, `.azure`, `.kube`, `keychains`, Windows DPAPI dirs) plus a prefix
  match against `SYSTEM_PREFIXES` (`/etc`, `/root`, `/boot`, `/proc`, `/sys`,
  `/system`, `C:\Windows`, `C:\Program Files[  (x86)]`, `C:\ProgramData`).
  It is unconditional; a `trusted_root` grant can never reach these paths.
  Gray-area directories (`/usr`, `/opt`, `/var`, `~/Library`) deliberately
  stay in the user-overridable `forbidden_paths` list instead, so a grant can
  still reach, for example, `/usr/local/...`.
- `classify_command`'s fail-closed floor ([`command_checks.rs`](./command_checks.rs)): a
  command that is not provably read-only (and not a recognized
  network/destructive command) is at least `CommandClass::Write`. The highest
  class across `;`/`|`/`&&`/`||`/newline-separated segments wins, and any
  unquoted redirect (`>`, `>>`) or `tee` lifts the whole command to at least
  `Write`. An LLM-declared category (`parse_declared_class`) is
  escalate-only: `agent/tinyagents/host/security_gate.rs` combines it as
  `class = check_gated_command(cmd)?.max(declared)`, so the model can raise
  the gate but never lower what the runtime determined.
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
- Hourly action budget. `ActionTracker` (`types.rs`) is a sliding
  one-hour window; `enforce_tool_operation`'s `Act` arm
  (`enforcement.rs`) calls `record_action` and refuses once the count exceeds
  `max_actions_per_hour`. `enforce_write_tier` alone (tier only, no budget) is
  used by callers that write at a finer grain than one tool call, for example
  the kernel memory guard under `MemoryCore::store`.

## Tests

- [`policy_tests.rs`](./policy_tests.rs), [`policy_allowlist_tests.rs`](./policy_allowlist_tests.rs), [`policy_injection_tests.rs`](./policy_injection_tests.rs),
  [`policy_paths_and_risk_tests.rs`](./policy_paths_and_risk_tests.rs), [`policy_trusted_roots_tests.rs`](./policy_trusted_roots_tests.rs), and
  [`policy_workspace_internal_tests.rs`](./policy_workspace_internal_tests.rs): behavior tests for classification,
  path checks, and tier enforcement.
- [`policy_disabled_tests.rs`](./policy_disabled_tests.rs): behavior with `[autonomy] enabled = false`.
- [`proptest_tests.rs`](./proptest_tests.rs): property tests over command classification.
- [`enforcement_scratch_dir_tests_tests.rs`](./enforcement_scratch_dir_tests_tests.rs): `openhuman_scratch_dir` is
  namespaced on every platform and `ensure_openhuman_scratch_dir` creates it.

## Further reading

- [Parent module (`security`)](../README.md)
- [Security architecture](../../../../../gitbooks/developing/architecture/security.md)
- [Privacy and security](../../../../../gitbooks/features/privacy-and-security.md)
- [Approval gate](../../../../../gitbooks/features/approval-gate.md)
- [OS keyring and secret storage](../../../../../gitbooks/features/os-keyring-and-secret-storage.md)
