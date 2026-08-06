# TinyAgents tool-model decision

**Status:** proposed at the WP-4 design gate; implementation requires explicit
approval.

This document resolves the choices left open by
`docs/tinyagents-port-plan.md` section 2. It deliberately does not authorize a
trait rewrite or builtin-tool port by itself.

## Decision summary

| Question | Decision |
| --- | --- |
| One tool trait everywhere? | No. Use native `tinyagents::Tool<State>` for generic crate tools and retain OpenHuman's `Tool` for product tools. `SharedToolAdapter` is the permanent boundary between them. |
| Where do `ToolResult` and `ToolContent` live? | Keep the MCP-style types in OpenHuman's always-compiled `skills::types`. Preserve them losslessly in TinyAgents `ToolResult::raw`; keep `content` as the model-facing rendering. |
| How is `ToolScope::AgentOnly` enforced? | Enforce scope at every exposure boundary: `All` everywhere, `AgentOnly` only in autonomous agent registries, and `CliRpcOnly` only in explicit CLI/RPC registries. Reject an out-of-scope direct invocation as well as hiding it from discovery. |
| Who owns security decisions? | OpenHuman. TinyAgents policy metadata is descriptive and provides generic fail-closed checks; OpenHuman still performs args-aware permission, approval, command, path, sandbox, credential, and external-effect gating. |

## 1. Trait boundary

OpenHuman should not mechanically migrate its roughly 200 product and
integration tools onto the generic crate trait. Its trait carries product
contracts that the crate does not: args-aware permission and approval,
generated-tool provenance, UI scope/category, markdown rendering, and
host-configured deadlines. Flattening these into static `ToolPolicy` fields is
lossy and risks bypassing security checks.

Instead:

- New or ported generic builtins implement `tinyagents::Tool<State>` in the
  crate.
- Product, RPC, dynamic integration, MCP, and OS-specific tools continue to
  implement OpenHuman `Tool`.
- `SharedToolAdapter` remains the supported host-to-harness boundary. Rename it
  only if a clearer public integration name is useful; do not delete it as
  migration debris.
- Add the inverse adapter only when an upstream builtin must be exposed through
  OpenHuman CLI/RPC. It must preserve the same scope and security gates as a
  native host tool.

This is convergence by ownership, not by forcing unrelated tools through one
trait.

## 2. Result ownership and lossless conversion

`skills::types::{ToolResult, ToolContent}` is an inert, dependency-light wire
type used by MCP, Node/Python runtimes, persisted surfaces, and nearly every
host tool. Moving it would create a large feature-gate and serialization blast
radius for no execution benefit. It remains compiled with the `skills` feature
both on and off.

The bridge must stop discarding structure:

1. Render `OpenHuman ToolResult::output_for_llm(true)` into TinyAgents
   `ToolResult::content`.
2. Serialize the complete OpenHuman result into TinyAgents `ToolResult::raw`,
   including `content`, `is_error`, and `markdownFormatted`.
3. Populate TinyAgents `error` when the host result reports `is_error`, without
   changing the structured `raw` value.
4. For a crate-native tool exposed back to a host surface, prefer a recognized
   OpenHuman-result envelope in `raw`; otherwise map a JSON `raw` value to
   `ToolContent::Json` and textual `content` to `ToolContent::Text`.

The envelope is versioned by its existing serde field names; no duplicate
`ToolContent` enum should be added to TinyAgents. Tests must cover mixed text
and JSON blocks, markdown, tool-reported errors, and the fallback for a
crate-native raw JSON value.

## 3. Scope semantics

`ToolScope` is host product policy and stays out of the crate. Its enforced
matrix is:

| Scope | Agent model | Explicit CLI/RPC |
| --- | ---: | ---: |
| `All` | yes | yes |
| `AgentOnly` | yes | no |
| `CliRpcOnly` | no | yes |

Filtering discovery is necessary but insufficient. The final dispatcher must
repeat the check so a caller cannot invoke a hidden tool by name. Rejections
must use the existing host policy/error shape and must not execute the tool.

The first implementation slice should inventory agent, CLI, JSON-RPC, Node,
Rhai, MCP, and generated-tool entrypoints, then add table-driven tests for the
matrix at each shared registry/dispatcher boundary. Until that inventory is
green, `AgentOnly` remains documented as unenforced and no tool should rely on
it as a security boundary.

## 4. Security and access mapping

`ToolPolicy` remains useful registry metadata, but it cannot replace
OpenHuman's live policy:

- Static host permission maps conservatively into TinyAgents side-effect and
  access metadata.
- Args-aware `permission_level_with_args`, `external_effect_with_args`, and
  generated-tool policy run at call time in host middleware before execution.
- `classify_command`, approval, workspace-internal fail-closed checks,
  `trusted_roots`, and sandbox selection remain host-owned.
- Per-call timeout resolution remains in the adapter until TinyAgents accepts a
  dynamic timeout hook without duplicating enforcement.
- File read-before-write tracking remains host middleware. It is not part of a
  generic tool result or policy type.

Filesystem builtins may move only when their crate implementation depends on
`WorkspaceDescriptor`/`ToolAccess` and the host can inject its stricter path
checks. Shell, Node, and package execution remain deferred until the crate has
an application-supplied command-classification/gating hook.

## 5. Implementation slices after approval

1. **Lossless result bridge:** populate and round-trip `ToolResult::raw`; add
   focused adapter tests. No registry changes.
2. **Scope enforcement:** audit exposure/invocation boundaries and enforce the
   matrix with table-driven tests. No trait changes.
3. **Native builtin registration seam:** add a host wrapper for selected
   crate-native time tools, proving bidirectional result and policy mapping.
4. **Filesystem pilot:** upstream/cut over one read-only tool using injected
   workspace policy and edit-tracking middleware where applicable.
5. **Broader ports:** proceed family by family. Product tools remain behind the
   permanent adapter.

Each slice is independently revertible. Do not combine the 236-file result-type
consumer graph with a tool-family port.

## 6. Acceptance criteria

- No structured host result is lost at the TinyAgents boundary.
- `AgentOnly` and `CliRpcOnly` are both hidden and rejected outside their
  allowed surfaces.
- Existing OpenHuman approval, args-aware permission, sandbox, and path checks
  remain authoritative.
- Generic crate tools can be registered without implementing the host trait
  directly.
- Builds with `--no-default-features` retain the OpenHuman result types and
  compile.
- `SharedToolAdapter` remains until all host product tools cease to exist,
  which is not a goal of this migration.
