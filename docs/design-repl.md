# Design: `openhuman repl` — Interactive Shell for Core Flows & Skills

**Issue**: [tinyhumansai/openhuman#92](https://github.com/tinyhumansai/openhuman/issues/92)
**Status**: Design (pre-implementation)
**Date**: 2026-03-30

---

## 1. Problem Statement

Validating core behavior today requires either the **full Tauri stack** (UI + sidecar) or **hand-crafted JSON-RPC / curl commands**. Both are slow for iteration, especially for:

- **Skill authors** debugging QuickJS execution, tool wiring, and sandbox boundaries.
- **Core developers** exercising RPC controllers during development.
- **QA / onboarding** running reproducible smoke checks from a terminal.

A lightweight, terminal-first **read-eval-print loop** would make all three workflows significantly faster.

### Target Users

| User | Need |
|------|------|
| **Skill author** | Discover, start, inspect, and call skill tools without a UI |
| **Core developer** | Exercise any registered RPC method, inspect config/health |
| **QA / CI script** | Batch-run a sequence of commands, assert outputs |
| **New contributor** | Follow onboarding docs ("paste this in the REPL to see X") |

### Non-Goals (v1)

- **Shipping to end users** as a primary interface — this is a dev/test tool.
- **Chat / LLM interaction** — no agentic inference loop; use the desktop app.
- **Remote connections** — the REPL drives the local core directly (in-process), not a remote server.
- **Full TUI** (curses, panels, split panes) — keep it a single-line REPL with good completion.
- **Plugin authoring within the REPL** — skills are authored in JS files, not typed live.
- **Replacing `openhuman run`** — the server subcommand stays as-is; the REPL is a peer.

---

## 2. Command / UX Sketch

### Starting the REPL

```bash
openhuman repl                    # interactive mode
openhuman repl --verbose          # debug logging enabled
openhuman repl --eval 'health snapshot'   # evaluate one command, print, exit
echo 'config get' | openhuman repl --batch  # stdin batch mode (no prompt)
```

### Prompt

```
openhuman> _
```

Prompt changes to show context when relevant:

```
openhuman> skill start gmail
  skill:gmail running (3 tools)

openhuman> _
```

### Example Session: Listing and Invoking a Skill

```
openhuman> help
Commands:
  <namespace> <function> [--param value ...]   Call any registered controller
  call <method> [json]                         Raw JSON-RPC method call
  skill list                                   List discovered skills
  skill start <id>                             Start a skill instance
  skill stop <id>                              Stop a running skill
  skill status <id>                            Inspect runtime state
  skill tools <id>                             List tools from a running skill
  skill call <id> <tool> [json-args]           Invoke a skill tool
  schema [namespace]                           Show controller schemas
  namespaces                                   List all namespaces
  env                                          Show workspace & runtime paths
  .verbose on|off                              Toggle debug logging
  .json on|off                                 Toggle raw JSON output
  .time on|off                                 Toggle timing display
  exit | quit | Ctrl-D                         Exit

openhuman> skill list
  ID          NAME                STATUS     TOOLS
  gmail       Gmail               pending    -
  notion      Notion              pending    -
  calendar    Google Calendar      pending    -

openhuman> skill start gmail
  skill:gmail initializing...
  skill:gmail running (3 tools)

openhuman> skill tools gmail
  TOOL                    DESCRIPTION
  gmail__search_emails    Search emails by query
  gmail__send_email       Send an email
  gmail__get_thread       Get full thread by ID

openhuman> skill call gmail search_emails {"query": "from:alice", "max_results": 5}
  {
    "content": [{ "type": "text", "text": "[...results...]" }],
    "is_error": false
  }
  (234ms)

openhuman> skill stop gmail
  skill:gmail stopped
```

### Example Session: Non-Skill Flow (Config + Health)

```
openhuman> config get
  {
    "workspace_dir": "/Users/dev/.openhuman/workspace",
    "model_settings": { "model_id": "neocortex-mk1", ... },
    ...
  }

openhuman> health snapshot
  {
    "uptime_secs": 42,
    "skills_running": 1,
    ...
  }

openhuman> config get_runtime_flags
  {
    "browser_allow_all": false,
    "local_ai_enabled": false,
    ...
  }
```

### Example: Raw JSON-RPC Style

```
openhuman> call openhuman.encrypt_secret {"plaintext": "my-api-key"}
  {
    "ciphertext": "enc:v1:..."
  }
```

### Example: Scriptable / Batch Mode

```bash
# One-liner for CI
openhuman repl --eval 'health snapshot' | jq '.uptime_secs'

# Batch script
cat <<'EOF' | openhuman repl --batch
config get_runtime_flags
skill list
health snapshot
EOF
```

---

## 3. Architecture

### 3.1 How the REPL Reuses Core Code

The REPL is **not** a second implementation. It drives the **same code paths** the JSON-RPC server uses:

```
┌────────────────────────────┐
│      openhuman repl        │
│  (rustyline read loop)     │
│                            │
│  parse_line()              │
│       │                    │
│       ▼                    │
│  ┌──────────────────┐      │
│  │ Skill shorthand?  │─yes──┼──► RuntimeEngine API (in-process)
│  │ (skill list, etc) │      │     engine.discover_skills()
│  └────────┬─────────┘      │     engine.start_skill()
│           │ no              │     engine.call_tool()
│           ▼                 │     engine.list_skills()
│  ┌──────────────────┐      │
│  │ Meta command?     │─yes──┼──► .verbose, .json, env, help (local)
│  │ (.verbose, help)  │      │
│  └────────┬─────────┘      │
│           │ no              │
│           ▼                 │
│  ┌──────────────────┐      │
│  │ namespace func    │──────┼──► invoke_method(state, method, params)
│  │ or call <method>  │      │     ↓ (same path as JSON-RPC server)
│  └──────────────────┘      │     all::try_invoke_registered_rpc()
│                            │     → domain handler
└────────────────────────────┘
```

**Key reuse points in existing code:**

| What | Where | How REPL uses it |
|------|-------|-----------------|
| Controller registry | `src/core/all.rs` | `all_controller_schemas()`, `schema_for_rpc_method()`, `try_invoke_registered_rpc()` |
| Method invocation | `src/core/jsonrpc.rs` | `invoke_method(state, method, params)` — identical call to server |
| Param parsing & validation | `src/core/cli.rs` + `all.rs` | `parse_function_params()`, `validate_params()` |
| Schema grouping / help | `src/core/cli.rs` | `grouped_schemas()`, `print_namespace_help()` |
| Skills runtime | `src/openhuman/skills/qjs_engine.rs` | `RuntimeEngine` — `discover_skills()`, `start_skill()`, `call_tool()`, `list_skills()` |
| Skill bootstrap | `src/core/jsonrpc.rs` | `bootstrap_skill_runtime()` — reused to init QuickJS engine |
| Default state | `src/core/jsonrpc.rs` | `default_state()` → `AppState` |

**No logic duplication.** The REPL is a thin input/output layer on top of the same internal APIs. Skill policy, parameter validation, secret redaction — all handled by the existing layers.

### 3.2 Workspace and Skills Registry Interaction

On startup the REPL:

1. Resolves workspace from `OPENHUMAN_WORKSPACE` env or `~/.openhuman` (same as server).
2. Calls `bootstrap_skill_runtime()` to initialize the `RuntimeEngine`, skills data dir, cron/ping schedulers — identical to server startup but **without** binding an HTTP port.
3. Skills source dir is resolved the same way (bundled `skills/skills/` or workspace).
4. All `skill *` commands go through the global `RuntimeEngine` singleton (set by `set_global_engine()`).

```
openhuman repl
  ├─ init logging (RUST_LOG or --verbose)
  ├─ bootstrap_skill_runtime()       ← same as server
  │    ├─ resolve workspace
  │    ├─ create RuntimeEngine
  │    ├─ set_global_engine()
  │    └─ start cron + ping schedulers
  ├─ create Tokio runtime
  ├─ create rustyline Editor (history, completer)
  └─ loop { readline → parse → invoke → print }
```

### 3.3 QuickJS Runtime Lifecycle

Skills run in QuickJS via `QjsSkillInstance`. The REPL shares the **same lifecycle** as the server:

- `skill start <id>` → `RuntimeEngine::start_skill()` → spawns QuickJS isolate + message loop.
- `skill call <id> <tool> <args>` → `RuntimeEngine::call_tool()` → sends `SkillMessage::CallTool` to the instance's mpsc channel → JS executes → returns `ToolCallResult`.
- `skill stop <id>` → sends `SkillMessage::Stop` → QuickJS context dropped.
- On REPL exit → all running instances are stopped (graceful shutdown).

No separate QuickJS management code is needed.

### 3.4 New Code Location

```
src/core/
  repl.rs          # REPL loop, line parsing, completion, formatting
  cli.rs           # Add "repl" match arm in run_from_cli_args()
  mod.rs           # Add `pub mod repl;`
```

Single new file (`repl.rs`, ~300-500 lines estimated). The `cli.rs` change is a one-line match arm.

### 3.5 New Dependency

```toml
# Cargo.toml
rustyline = { version = "15", features = ["with-file-history"] }
```

`rustyline` provides: line editing, history (persisted to `~/.openhuman/repl_history`), tab completion, Ctrl-C/Ctrl-D handling, and cross-platform terminal support (including Windows).

---

## 4. Safety: Secrets, Tokens, and PII

### Principles

1. **No secrets in REPL output.** The REPL displays results from `invoke_method()` which already passes through the same code as the server. Existing RPC handlers are responsible for not returning raw secrets.

2. **Input redaction.** The REPL must **not** log user-typed input at `info` level or above if it may contain secrets (e.g., `--api_key`, `--plaintext`). Debug logging of input is gated behind `--verbose` / `RUST_LOG=debug`.

3. **History file exclusions.** Lines matching sensitive patterns are **not written** to the history file:
   - Any line containing `api_key`, `token`, `secret`, `password`, `plaintext`, `mnemonic`
   - Raw JSON with fields named `*key`, `*token`, `*secret`

4. **Skill output.** `call_tool()` returns `ToolCallResult { content, is_error }`. The REPL prints `content` as-is. Skill authors are responsible for not leaking credentials in tool output (same as in the desktop app). The REPL adds a one-line warning on first `skill call`:
   ```
   note: skill tool output is printed verbatim; ensure skills do not emit secrets
   ```

5. **No JWT / session tokens.** The REPL operates **in-process** with no auth layer. There are no session tokens to leak. Backend API calls (if any skill makes them) use credentials stored in the skills data dir, not typed interactively.

### Implementation

```rust
fn should_skip_history(line: &str) -> bool {
    let lower = line.to_lowercase();
    const SENSITIVE: &[&str] = &[
        "api_key", "token", "secret", "password",
        "plaintext", "mnemonic", "private_key",
    ];
    SENSITIVE.iter().any(|s| lower.contains(s))
}
```

---

## 5. Tab Completion

`rustyline` supports custom completers. The REPL provides context-aware completion:

| Position | Completes |
|----------|-----------|
| First word | Namespace names, `call`, `skill`, `schema`, `namespaces`, `env`, `help`, `exit` |
| After namespace | Function names within that namespace |
| After `skill` | `list`, `start`, `stop`, `status`, `tools`, `call` |
| After `skill start/stop/status/tools` | Discovered skill IDs |
| After `skill call <id>` | Tool names from that skill's snapshot |
| After function name | `--param_name` flags from schema |

Completion data is derived from `all_controller_schemas()` and `RuntimeEngine::list_skills()` — no extra state.

---

## 6. Output Modes

| Mode | Default | Toggle | Behavior |
|------|---------|--------|----------|
| **Pretty** | on | `.json off` | Colored, indented JSON with field highlights |
| **Raw JSON** | off | `.json on` | Machine-parseable `serde_json::to_string_pretty` |
| **Timing** | off | `.time on` | Appends `(Xms)` after each result |
| **Verbose** | off | `.verbose on` | Sets `RUST_LOG=debug` for the process |

In `--batch` / `--eval` mode, output defaults to raw JSON (machine-friendly).

---

## 7. Error Handling

```
openhuman> config update_model_settings
  error: missing required param 'model_id'
  hint: openhuman config update_model_settings --help

openhuman> skill start nonexistent
  error: skill 'nonexistent' not found
  hint: run `skill list` to see discovered skills

openhuman> skill call gmail bad_tool {}
  error: tool 'bad_tool' not found in skill 'gmail'
  hint: run `skill tools gmail` to see available tools
```

Errors print to stderr (red if tty), results to stdout. This makes `--eval` / `--batch` output clean for piping.

---

## 8. Follow-Up: Implementation Milestones

### Phase 1 — MVP REPL (single PR)

- [ ] `openhuman repl` subcommand with rustyline loop
- [ ] Namespace/function dispatch via `invoke_method()`
- [ ] `call <method> [json]` for raw RPC
- [ ] `help`, `schema`, `namespaces`, `env`, `exit`
- [ ] `.json`, `.verbose`, `.time` toggles
- [ ] History file with sensitive-line exclusion
- [ ] Basic tab completion (namespaces + functions)
- [ ] `--eval` single-command mode
- [ ] Unit tests for line parsing and completion

**Issue**: to be created after design approval.

### Phase 2 — Skills-Focused Commands

- [ ] `skill list/start/stop/status/tools/call` shorthand commands
- [ ] Tab completion for skill IDs and tool names
- [ ] Skill event streaming (print `skill-state-changed` events inline)
- [ ] `skill setup <id>` interactive setup flow
- [ ] Skill output formatting (tool result → readable text)

**Issue**: to be created after Phase 1 merges.

### Phase 3 — Script Mode & CI

- [ ] `--batch` stdin mode (one command per line, raw JSON output)
- [ ] Exit codes: 0 = all ok, 1 = any command failed
- [ ] `--eval` supports semicolon-separated commands
- [ ] Example scripts in `docs/` for common workflows
- [ ] CI integration example (smoke test in GitHub Actions)

**Issue**: to be created after Phase 2.

---

## Appendix: Alternatives Considered

### A. REPL as a wrapper around HTTP (like curl)

**Rejected.** Adds network overhead, requires `openhuman run` to be running, and can't access the skill runtime's in-process state without the server. In-process invocation is simpler and faster.

### B. Embed a full Lua/Python scripting layer

**Rejected for v1.** Over-engineered for the stated goals. The `--eval` / `--batch` modes give enough scriptability. Can revisit if demand appears.

### C. TUI with panels (like `lazygit`)

**Rejected for v1.** Adds significant complexity (UI framework, layout, event handling). A line-based REPL covers the stated use cases. A TUI could be built on top later if warranted.

### D. Use `clap` derive macros for REPL parsing

**Rejected.** `clap` is designed for one-shot CLI parsing, not interactive loops. The existing hand-rolled parser in `cli.rs` is a better fit; the REPL reuses `parse_function_params()` directly.
