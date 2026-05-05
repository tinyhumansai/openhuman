# Coding-harness tool surface

OpenHuman exposes a coherent baseline of code-focused tools to its
agents. This page is the canonical map: which tool the model should
reach for, what permissions it needs, and where the implementation
lives.

It is intentionally a flat catalog — not a guide on how the agent
loop dispatches tools. For that, see
[`docs/ARCHITECTURE.md`](ARCHITECTURE.md) and
[`src/openhuman/tools/`](../src/openhuman/tools/).

Tracking issue: [#1205](https://github.com/tinyhumansai/openhuman/issues/1205).

## Surface at a glance

| Category | Tool | Permission | Source |
| --- | --- | --- | --- |
| **Navigation** | `file_read` | ReadOnly | `tools/impl/filesystem/file_read.rs` |
| | `grep` | ReadOnly | `tools/impl/filesystem/grep.rs` |
| | `glob` | ReadOnly | `tools/impl/filesystem/glob_search.rs` |
| | `list` | ReadOnly | `tools/impl/filesystem/list_files.rs` |
| **Editing** | `file_write` | Write | `tools/impl/filesystem/file_write.rs` |
| | `edit` | Write | `tools/impl/filesystem/edit_file.rs` |
| | `apply_patch` | Write | `tools/impl/filesystem/apply_patch.rs` |
| **Execution** | `shell` | Execute | `tools/impl/system/shell.rs` |
| **Interaction** | `ask_clarification` (`question`) | None | `tools/impl/agent/ask_clarification.rs` |
| | `spawn_subagent` (`task`) | varies | `tools/impl/agent/spawn_subagent.rs` |
| | `todowrite` | None | `tools/impl/agent/todo_write.rs` |
| | `plan_exit` | None | `tools/impl/agent/plan_exit.rs` |
| **Web research** | `web_search` (`websearch`) | ReadOnly | `tools/impl/network/web_search.rs` |
| | `web_fetch` (`webfetch`) | ReadOnly | `tools/impl/network/web_fetch.rs` |
| | `http_request`, `curl` (richer HTTP) | ReadOnly | `tools/impl/network/` |
| **Code intel** | `lsp` *(capability-gated)* | ReadOnly | `tools/impl/system/lsp.rs` |

Names in parentheses are the canonical coding-harness names from issue #1205.
The Rust struct and registration name match the column on the left; the
alias in parentheses is the conceptual role.

## What each tool does

### Navigation

- **`file_read { path }`** — read a workspace-relative file (≤10 MB).
  Path-sandboxed and symlink-escape blocked.
- **`grep { pattern, path?, max_matches?, case_insensitive? }`** —
  regex search across files. Returns `path:line:text` lines, capped.
  Skips `.git`, `node_modules`, `target`, `.next`, `dist`, `build`,
  `.cache`.
- **`glob { pattern, max_results? }`** — list files matching a glob
  (e.g. `src/**/*.rs`). Sorted newest-first. Same skip set as `grep`.
- **`list { path? }`** — non-recursive directory listing. Each line is
  `<kind>\t<name>` where kind is `dir`, `file`, or `link`.

### Editing

- **`file_write { path, content }`** — overwrite (or create) a file.
- **`edit { path, old_string, new_string, replace_all? }`** — exact
  string-replace. By default `old_string` must be unique in the file
  (so the model can't accidentally rewrite every occurrence); set
  `replace_all` to override.
- **`apply_patch { edits[] }`** — atomic batch of `edit` operations
  across one or more files. Validation runs over the whole batch
  first; if any edit fails (path not allowed, non-unique match, file
  too large, …) **no** files are written.

### Execution

- **`shell { command }`** — run a vetted shell command. Use this only
  when the right primitive doesn't already exist (e.g. `grep` should
  almost always replace `shell { command: "grep ..." }`).

### Interaction & control flow

- **`ask_clarification` (canonical `question`)** — pause the run and
  ask the user a structured question. Resumes with the answer once
  the user replies.
- **`spawn_subagent` (canonical `task`)** — delegate a focused unit of
  work to a child agent. Returns a single text result.
- **`todowrite { todos[] }`** — replace the agent's lightweight todo
  list. Each item is `{content, status}` where `status ∈
  {pending, in_progress, completed}`. Only one `in_progress` allowed.
- **`plan_exit { plan }`** — emit a `[plan_exit]` marker plus the
  plan text, signaling that the plan-mode pass is done and the
  harness should hand off to a build-mode pass. The plan→build
  switch on the harness side is follow-up work; the marker is stable
  today so prompts can be written against it.

### Web research

- **`web_search` (canonical `websearch`)** — backend-proxied search
  via Parallel. Returns ranked excerpts.
- **`web_fetch` (canonical `webfetch`)** — single-purpose `GET` →
  text body, capped. Reuses the same `allowed_domains` gate as
  `http_request`. Reach for `web_fetch` when reading docs/READMEs;
  reach for `http_request` only when you need methods, headers, or
  a body.

### Code intelligence

- **`lsp { kind, language, file, line?, character?, symbol? }`** —
  capability-gated. Registered only when `OPENHUMAN_LSP_ENABLED=1`.
  Schema is stable; the language-server backend is a follow-up — the
  current implementation returns a clear `not yet implemented` error
  when called, so callers can feature-detect.

## Permissions and modes

Permissions live on the [`Tool`
trait](../src/openhuman/tools/traits.rs). Each tool returns one of:
`None`, `ReadOnly`, `Write`, `Execute`, `Dangerous`. Channels can set
a maximum permission level — anything above is rejected before
execution.

Today's coding-harness mapping:

| Permission | Tools |
| --- | --- |
| **None** | `todowrite`, `plan_exit`, `ask_clarification` |
| **ReadOnly** | `file_read`, `grep`, `glob`, `list`, `web_search`, `web_fetch`, `http_request`, `lsp` |
| **Write** | `file_write`, `edit`, `apply_patch` |
| **Execute** | `shell` |

### Plan mode vs build mode

`plan_exit` is the seam where a plan-mode pass hands off to a
build-mode pass. The marker (`[plan_exit]`) is stable; the harness
that consumes the marker and switches modes is a follow-up. Until
that lands, a single agent can still call `plan_exit` to log the
plan and then proceed to execution in the same pass.

In a future plan-mode runner, the rules will be:

- Plan mode allows only `None` and `ReadOnly` tools, plus
  `plan_exit`.
- Build mode allows the full surface (subject to the channel cap).

The permission machinery is already in place; only the mode-runner
wrapper is missing.

## When to add a new tool

If the question is "should this be a new tool or just `shell` with
a longer command?", prefer a new tool when:

- The operation is one the model gets wrong from `shell` (e.g.
  pattern-matching tasks where regex syntax differs across
  platforms).
- The operation needs structured input/output the LLM can rely on
  (e.g. `apply_patch`'s atomic semantics).
- The operation has security gates that are easier to enforce in
  Rust than in shell quoting (e.g. `web_fetch`'s allowed-domains
  gate).

If the answer is yes, follow the existing pattern under
`src/openhuman/tools/impl/<category>/`, register in
`src/openhuman/tools/ops.rs`, and add unit tests in the same file.

## Follow-up work

These items from issue #1205 are explicitly out of scope for this
baseline PR:

- Plan-mode and build-mode runners that consume `[plan_exit]`.
- Child-session-backed `task` execution with stable session ids.
- Real LSP backend behind the `lsp` tool.
- Richer permission model (per-tool channel allowlists, per-call
  approval policies).
- Controller-registry exposure of the new agent-only tools to
  JSON-RPC. Today they remain agent-only — the registry already
  exposes `tools_web_search` (and friends) where the Tauri shell
  needs them.
