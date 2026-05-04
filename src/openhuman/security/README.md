# Security

Trust boundary for the autonomous core. Owns the autonomy / risk policy, sandbox backends (Docker, Bubblewrap, Firejail, Landlock, Noop), the audit log of agent actions, the encrypted secret store, the public-bind / pairing guard, and the `redact()` helper used for safe logging. Does NOT own the cross-domain `EncryptionEngine` (lives in `encryption/`) or per-channel credential storage (`credentials/`).

## Public surface

- `pub struct SecurityPolicy` — `policy.rs` — assemble runtime policy from `AutonomyConfig` + workspace dir.
- `pub enum AutonomyLevel` — `policy.rs` — `Supervised` / `SemiAutonomous` / `Autonomous`.
- `pub enum CommandRiskLevel` / `pub enum ToolOperation` / `pub struct ActionTracker` — `policy.rs` — risk classification and per-session tracking.
- `pub trait Sandbox` / `pub struct NoopSandbox` — `traits.rs` — pluggable sandbox abstraction.
- `pub fn create_sandbox(config: &SecurityConfig) -> Arc<dyn Sandbox>` — `detect.rs:1` — pick the best backend for the host.
- Sandbox backends: `pub mod docker`, `pub mod bubblewrap`, `pub mod firejail`, `pub mod landlock` — domain-specific implementations of `Sandbox`.
- `pub struct SecretStore` — `secrets.rs` — XOR / OS-keychain encrypted secret persistence with round-trip helpers.
- `pub struct AuditLogger` / `pub enum AuditEventType` / `pub struct AuditEvent` / `pub struct Actor` / `pub struct Action` / `pub struct ExecutionResult` / `pub struct SecurityContext` / `pub struct CommandExecutionLog` — `audit.rs` — append-only audit trail.
- `pub struct PairingGuard` / `pub fn constant_time_eq` / `pub fn is_public_bind` — `pairing.rs` — pairing-token check before binding the RPC server publicly.
- `pub fn redact(value: &str) -> String` — `core.rs:3` — uniform 4-char-prefix redaction for logs.
- `pub fn security_policy_info() -> RpcOutcome<serde_json::Value>` — `ops.rs` — RPC handler used by the doctor / settings UI.

## Calls into

- `src/openhuman/config/` — `SecurityConfig`, `AutonomyConfig` for policy + sandbox selection.
- OS-level sandbox tools — `docker`, `bwrap`, `firejail`, `landlock` syscalls (per backend).
- Filesystem under the workspace dir for the audit log + secrets store.

## Called by

- `src/openhuman/cron/scheduler.rs` — wraps shell jobs in `SecurityPolicy::from_config`.
- `src/openhuman/tools/local_cli.rs`, `tools/ops.rs`, and most `tools/impl/{system,network,memory,agent}/*.rs` — every executable tool consults `SecurityPolicy`.
- `src/openhuman/tools/impl/network/{curl,http_request,composio}.rs` — risk-classify outbound calls.
- `src/openhuman/tools/impl/memory/{store,forget}.rs` — sensitive-write tracking.
- `src/openhuman/tools/impl/agent/delegate.rs` — sub-agent dispatch goes through autonomy gate.
- `src/openhuman/credentials/` — uses `SecretStore` and `redact`.

## Tests

- Unit: `pairing_tests.rs`, `policy_tests.rs`, `secrets_tests.rs`.
- `core.rs` `#[cfg(test)] mod tests` — round-trips `SecretStore` encrypt/decrypt, `redact()` cases, `PairingGuard` defaults.
- Sandbox-backend smoke: each backend file has its own `#[cfg(test)]` blocks where the binary is available.
