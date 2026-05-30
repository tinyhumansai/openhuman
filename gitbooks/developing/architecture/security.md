---
description: >-
  Trust boundary for the autonomous core - autonomy / risk policy, pluggable
  sandbox backends (Docker, Bubblewrap, Firejail, Landlock, Noop), audit log,
  encrypted secret store, public-bind / pairing guard, and the redact() helper.
icon: shield-halved
---

# Security (`src/openhuman/security/`)

`src/openhuman/security/` is the **trust boundary for the autonomous core**. It owns the autonomy / risk policy that decides whether a given tool call is allowed, the pluggable sandbox backends that confine those calls when the host supports it, the append-only audit log of every agent action, the encrypted secret store, the pairing guard that gates public binding of the RPC server, and the `redact()` helper every other domain uses to keep logs free of plaintext credentials.

It does **not** own:

- The cross-domain `EncryptionEngine` — that lives in `src/openhuman/encryption/`.
- Per-channel credential storage — that lives in `src/openhuman/credentials/`.

This module is the place to look first when asking "is this agent action allowed, and if so, how is it confined?"

## Public surface

| Item                                                                                                                          | File         | Purpose                                                                 |
| ----------------------------------------------------------------------------------------------------------------------------- | ------------ | ----------------------------------------------------------------------- |
| `SecurityPolicy`                                                                                                              | `policy.rs`  | Assembles runtime policy from `AutonomyConfig` + workspace dir.         |
| `AutonomyLevel` (`Supervised` / `SemiAutonomous` / `Autonomous`)                                                              | `policy.rs`  | Three-step autonomy ladder.                                             |
| `CommandRiskLevel`, `ToolOperation`, `ActionTracker`                                                                          | `policy.rs`  | Risk classification + per-session counting.                             |
| `Sandbox` trait, `NoopSandbox`                                                                                                | `traits.rs`  | The pluggable sandbox abstraction; every backend implements `Sandbox`.  |
| `create_sandbox(&SecurityConfig) -> Arc<dyn Sandbox>`                                                                         | `detect.rs`  | Picks the best backend available on the host at runtime.                |
| `pub mod docker / bubblewrap / firejail / landlock`                                                                           | (siblings)   | Per-backend implementations of `Sandbox`.                               |
| `SecretStore`                                                                                                                 | `secrets.rs` | XOR / OS-keychain encrypted secret persistence with round-trip helpers. |
| `AuditLogger`, `AuditEventType`, `AuditEvent`, `Actor`, `Action`, `ExecutionResult`, `SecurityContext`, `CommandExecutionLog` | `audit.rs`   | Append-only audit trail.                                                |
| `PairingGuard`, `constant_time_eq`, `is_public_bind`                                                                          | `pairing.rs` | Pairing-token check before binding the RPC server publicly.             |
| `redact(value: &str) -> String`                                                                                               | `core.rs`    | Uniform 4-char-prefix redaction for logs.                               |
| `security_policy_info() -> RpcOutcome<serde_json::Value>`                                                                     | `ops.rs`     | RPC handler for the doctor / settings UI.                               |

## Sandbox backend selection

`detect::create_sandbox` walks a preference list and returns the **first available** backend on the host. The exact order is encoded in `detect.rs`; in practice it favours the strongest available isolation:

```text
                ┌──────────────┐
SecurityConfig ─►│ create_sandbox│
                └──────┬───────┘
                       │ probes
                       ├─► Docker      (best isolation; needs daemon)
                       ├─► Bubblewrap  (Linux user-namespace sandbox)
                       ├─► Firejail    (Linux setuid sandbox)
                       ├─► Landlock    (Linux LSM; in-process)
                       └─► Noop        (last resort; logs only)
```

The agent never sees the choice — it just calls into `Sandbox::run(...)` and the active backend handles the rest. Every backend lives in a sibling file (`docker.rs`, `bubblewrap.rs`, `firejail.rs`, `landlock.rs`); the noop fallback is in `traits.rs`.

## Autonomy ladder

`AutonomyLevel` is a three-step ladder that controls how aggressively the policy gates tool calls:

- **Supervised** — every higher-risk tool call requires an explicit approval round-trip.
- **SemiAutonomous** — low / medium-risk tool calls flow through; higher-risk ones still approval-gate.
- **Autonomous** — the policy lets the agent run unattended within budget and risk caps.

`CommandRiskLevel` + `ToolOperation` classify a given tool call; `ActionTracker` keeps the per-session counts that the policy compares against caps. The agent harness asks `SecurityPolicy` for a decision before every executable tool dispatch.

## Audit log

`audit.rs` writes an append-only stream of `AuditEvent`s under the workspace dir. Every executable tool call lands here with its `Actor` (agent / user), `Action`, `ExecutionResult`, and the `SecurityContext` (autonomy level, sandbox backend, etc.) it ran under. The log is the post-hoc story of what the agent did and why it was allowed.

## Pairing guard

`PairingGuard` (in `pairing.rs`) stands between the RPC server and any attempt to bind to a non-loopback address. `is_public_bind` detects the dangerous case; `PairingGuard` requires a constant-time-compared pairing token (`constant_time_eq`) before such a bind is permitted. This is the iOS / LAN-companion pairing flow's defence against an unpaired peer attaching to the desktop core.

## Secret store

`SecretStore` (in `secrets.rs`) persists per-key secrets with at-rest encryption. On supported platforms the encryption key comes from the OS keychain; otherwise it falls back to a workspace-local XOR scheme (which is **obfuscation, not security**, and is documented as such in the source).

## `redact()`

`redact(value)` returns a uniform 4-char-prefix string (e.g. `"sk-a"` -> `"sk-a…"`) for use in logs and error messages. Use it whenever a secret, credential, token, or PII string is about to be formatted into a `log::` / `tracing::` call. Other domains call it directly — `credentials/`, `webhooks/`, `composio/`, the integration adapters.

## Layout

| Path                                                          | Role                                                                      |
| ------------------------------------------------------------- | ------------------------------------------------------------------------- |
| `policy.rs`, `policy_tests.rs`                                | `SecurityPolicy`, `AutonomyLevel`, risk classification, action tracking.  |
| `traits.rs`                                                   | `Sandbox` trait + `NoopSandbox` fallback.                                 |
| `detect.rs`                                                   | `create_sandbox` — best-available-backend selection.                      |
| `docker.rs` / `bubblewrap.rs` / `firejail.rs` / `landlock.rs` | Per-backend `Sandbox` implementations.                                    |
| `core.rs`                                                     | `redact()` + small shared helpers (has its own `#[cfg(test)] mod tests`). |
| `audit.rs`                                                    | Append-only audit log types.                                              |
| `secrets.rs`, `secrets_tests.rs`                              | `SecretStore` + round-trip tests.                                         |
| `pairing.rs`, `pairing_tests.rs`                              | `PairingGuard` + constant-time helpers.                                   |
| `ops.rs`                                                      | RPC handler (`security_policy_info`).                                     |
| `schemas.rs`                                                  | Controller schemas + handler dispatch.                                    |
| `mod.rs`                                                      | Re-exports of the public surface above.                                   |

## Calls into

- `src/openhuman/config/` — `SecurityConfig`, `AutonomyConfig` for policy + sandbox selection.
- OS-level sandbox tools — `docker`, `bwrap`, `firejail`, Landlock syscalls (per backend).
- Workspace filesystem — for the audit log and secret store.

## Called by

- `src/openhuman/cron/scheduler.rs` — wraps shell jobs in `SecurityPolicy::from_config`.
- `src/openhuman/tools/local_cli.rs`, `tools/ops.rs`, and most `tools/impl/{system,network,memory,agent}/*.rs` — every executable tool consults `SecurityPolicy`.
- `src/openhuman/tools/impl/network/{curl,http_request,composio}.rs` — risk-classify outbound calls.
- `src/openhuman/memory/tools/{store,forget}.rs` — sensitive-write tracking.
- `src/openhuman/agent/tools/delegate.rs` — sub-agent dispatch goes through the autonomy gate.
- `src/openhuman/credentials/` — uses `SecretStore` and `redact`.

## Tests

- Unit: `pairing_tests.rs`, `policy_tests.rs`, `secrets_tests.rs`.
- `core.rs` has its own `#[cfg(test)] mod tests` — round-trips `SecretStore` encrypt / decrypt, `redact()` cases, `PairingGuard` defaults.
- Sandbox-backend smoke tests: each backend file has its own `#[cfg(test)]` blocks where the binary is available on the host.

## Related

- [`security/README.md`](https://github.com/tinyhumansai/openhuman/blob/main/src/openhuman/security/README.md) — authoritative internal-audience overview this page mirrors.
- [Architecture overview](../architecture.md) — wider system context.
- [Agent Harness](agent-harness.md) — where `SecurityPolicy` is consulted on every tool dispatch.
