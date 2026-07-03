# 01.1 — ToolPolicy round-trip

The spec's "SDK gap: ToolSchema has no metadata map" is obsolete: the crate
`Tool` trait has `policy() -> ToolPolicy` and `ToolRegistry::policies()`.

Current status (2026-07-02): adapters expose classified SDK policies, the
crate `ToolPolicyMiddleware` is installed from `harness.tools().policies()`, and
`ToolOutputMiddleware` now reads per-tool result caps from that registry
snapshot instead of rebuilding a `name -> Arc<dyn Tool>` lookup. The remaining
crate-internal OpenHuman lookups are behavior-preserving overlays for data the
crate policy snapshot cannot represent yet: args-aware external effects,
CLI/RPC-only scope, args-aware permission level, and generated-tool runtime
context. The local `tinyagents/middleware.rs` module is crate-internal; turn
config types stay crate-visible only through `openhuman::tinyagents` re-exports.
The `ToolAdapter`/`SharedToolAdapter` execution wrappers are crate-internal
implementation details of the shared runner.

## Steps

1. In `src/openhuman/tinyagents/tools.rs` (`ToolAdapter`/`SharedToolAdapter`),
   implement `policy()` by mapping OpenHuman trait methods
   (`src/openhuman/tools/traits.rs`): permission level → `ToolAccess
   { approval_required, background_safe }`; `external_effect` →
   `ToolSideEffects`; timeout policy → `ToolRuntime.timeout_ms`;
   concurrency safety → `ToolRuntime.idempotent`; `max_result_size_chars`
   → `ToolRuntime.max_result_bytes`; sandbox mode → `SandboxMode`.
   Every adapter-produced policy sets `classified: true`.
2. Partially done: rework `ApprovalSecurityMiddleware`, `CliRpcOnlyMiddleware`,
   `ToolPolicyMiddleware`, and `ToolOutputMiddleware`
   (`src/openhuman/tinyagents/middleware.rs`) to read `ToolPolicy` from the
   registry instead of the shared `name → Arc<dyn Tool>` side-lookup.
   `ToolOutputMiddleware` is registry-backed. Keep the crate-internal OpenHuman
   overlays the static crate policy cannot yet express: `external_effect_with_args`,
   `ToolScope::CliRpcOnly`, `permission_level_with_args`, and
   `generated_runtime_context`.
3. Partially done: install crate `ToolPolicyMiddleware` in
   `assemble_turn_harness` (`src/openhuman/tinyagents/mod.rs`) with
   `require_sandbox(true)` only. TinyAgents 1.3 exposes classification,
   approval, and result-byte gates, but OpenHuman still keeps args-aware
   approval/audit and legacy result-cap wording in local overlays. Enable the
   stricter crate gates only after those overlays have equivalent policy
   coverage or can be deleted. Assert every registered tool is classified in the
   adapter-inventory test when those gates become fail-closed.
4. Done for output budgeting: `TurnContextMiddleware.install` takes the SDK
   policy snapshot instead of the `&tool_sets` parameter.

## Deletions

- Deleted: `ToolOutputMiddleware`'s `name → Arc<dyn Tool>` policy snapshot in
  `tinyagents/middleware.rs`.
- Remaining as crate-internal overlays by design until the SDK policy surface
  grows equivalent metadata: `external_effect_with_args`, `ToolScope::CliRpcOnly`,
  `permission_level_with_args`, and `generated_runtime_context` overlays.
- Deleted: redundant per-call trait re-queries in the legacy
  `agent_tool_exec.rs` policy chain.

## Acceptance

- `ToolRegistry::policies()` snapshot test: all tools classified, policies
  serialize stably.
- Approval/security/output behavior parity tests still green
  (middleware suite ~22 tests).
