# 01 — Tooling

Move tool metadata, policy enforcement, unknown-tool recovery, dynamic
exposure, and output budgeting onto SDK primitives; delete the OpenHuman
side-lookup pattern and legacy tool plumbing.

Target SDK surface (available across tinyagents 1.2.x–1.3.0; current repo lock
is 1.3.0):

- `Tool::policy() -> ToolPolicy { side_effects, runtime, access }` —
  serializable safety metadata (read_only/writes_files/network/destructive/
  payment; timeout/retries/idempotent/sandbox/max_result_bytes; workspace
  access/trusted_roots/credentials/approval_required/background_safe).
- `ToolPolicyMiddleware` (fail-closed on unclassified), `ToolAllowlistMiddleware`,
  `DynamicToolSelectionMiddleware`, `ContextualToolSelectionMiddleware`,
  `HumanApprovalMiddleware`.
- `RunPolicy.unknown_tool: UnknownToolPolicy::{Fail, ReturnToolError, Rewrite}`.
- `ToolRegistry::policies()`, `ToolExecutionContext.workspace:
  WorkspaceDescriptor`.

Steps:

1. `01-tool-policy.md` — implement `policy()` on the adapters; retire the
   `name → Arc<dyn Tool>` side-lookup.
2. `02-unknown-tool.md` — replace the sentinel with `UnknownToolPolicy`.
3. `03-dynamic-exposure.md` — allowlists/denylists/channel ceiling as
   selection middleware.
4. `04-tool-output.md` — output budgets + payload summarizer as `after_tool`;
   delete legacy files.

Done when: no OpenHuman middleware queries tool trait methods ad hoc; the
sentinel is gone; tool visibility decisions are middleware-owned and
event-visible; deletions in `99-deletion-ledger.md` §tooling are done.
