# Plan: `openhuman tui` — feature-gated terminal chat UI

> Historical v1 plan. The shipped CLI now extends this foundation into a Logs-first four-tab UI
> (Logs, Chat, Config, Settings). Bare `openhuman` auto-launches only with terminal stdin/stdout on
> a non-container host; `--no-tui` suppresses that default and explicit `openhuman tui` still forces
> the UI. Config uses curated safe getters/updaters, and Settings uses registered auth controllers.

## Goal

Running `openhuman-core tui` (alias `chat`) opens a ratatui-based terminal UI that is an
interface into the general chat (the same `web_chat` surface the desktop app uses),
gated behind a Cargo feature `tui`.

## Architecture decisions (grounded in current code)

1. **Cargo feature**: `tui = ["dep:ratatui", "dep:crossterm"]`, added to `default`
   (repo convention: gates default-ON). It must NOT ship in the desktop app —
   add `'tui': 'Terminal UI subcommand; the desktop app ships its own Tauri UI.'`
   to `INTENTIONALLY_NOT_FORWARDED` in `scripts/ci/check-feature-forwarding.mjs`.
2. **Module**: crate-level `src/tui/`, gated at its declaration in `src/lib.rs`:
   - `mod.rs` always compiled; real submodules `#[cfg(feature = "tui")]`;
     `#[cfg(not(feature = "tui"))] mod stub;` exposing the same `run_from_cli`.
   - `stub.rs` `run_from_cli` bails with
     `"tui feature disabled at compile time … rebuild with --features tui"`
     (mirror `src/openhuman/mcp/server/stub.rs:42`).
   - No controllers, no agent tools, no `all.rs` changes (leaf client, like `flows`'
     philosophy: absence, not degraded registration — but here the only outside
     touch-point is the CLI arm, which uses the stub for a build-fact error).
3. **CLI arm**: in `src/core/cli.rs` match (~line 63), add
   `"tui" | "chat" => run_tui_from_cli(&args[1..])`.
   Arm stays **un-cfg'd** (mcp precedent). Add `"tui" | "chat"` to the banner-suppression
   `matches!` at lines 48–50 (a TUI must own the terminal).
4. **In-process core, no HTTP**: build a multi-thread tokio runtime with
   `AGENT_WORKER_STACK_BYTES` stack (copy `run_server_command` shape, cli.rs:219–311),
   then `CoreBuilder::new(HostKind::Cli).domains(DomainSet::full()).services(ServiceSet::none()).build()`.
   `channel.web_chat` needs `DomainGroup::Channels`, so `harness()` is not enough.
5. **Chat flow**:
   - `client_id = "tui-<random hex>"`.
   - Threads via `runtime.invoke("threads.list"| "threads.create_new", …)`;
     CLI flags: `--thread <id>`, `--new` (default: create new thread).
   - Send turn: `runtime.invoke("channel.web_chat", {client_id, thread_id, message, …})`
     (schema: `src/openhuman/web_chat/schemas.rs:45`; ops entry `ops.rs:1199`/`start_chat` at 391).
   - Stream: drain `web_chat::subscribe_web_channel_events()` (broadcast bus,
     `src/openhuman/web_chat/event_bus.rs:14`), filter by our `client_id`.
     Render `text_delta`/`thinking_delta` (`delta`, `delta_kind` fields on
     `WebChannelEvent`, `src/core/socketio.rs:98`), show `tool_call`/`tool_result`
     as status lines, finish on `chat_done` (use `full_response` as authoritative
     final text) or `chat_error` (show `message`).
   - Cancel in-flight turn: `channel.web_cancel` on Esc.
6. **UI v1 scope** (keep it tight):
   - Alternate screen + raw mode; transcript viewport with scrollback
     (PgUp/PgDn/mouse optional), single-line input box, status bar
     (thread id, model/turn state, key hints), spinner while streaming.
   - Keys: Enter=send, Esc=cancel turn, Ctrl+N=new thread, Ctrl+C/Ctrl+D=quit.
   - Distinguish user / assistant / thinking (dim) / tool activity / errors.
     Ocean-ish accent per design tokens is fine but keep it terminal-native.
7. **Terminal hygiene (critical)**:
   - Panic hook + Drop guard that restores the terminal (leave raw mode,
     LeaveAlternateScreen) before the panic message prints.
   - **Logging must not hit stdout/stderr while the TUI owns the terminal** —
     inspect how `run_from_cli`/`run_server_command` init logging
     (`src/core/logging.rs`) and route core logs to file only (or suppress console)
     for the tui arm. Core boot logs corrupting the UI is a bug.
8. **Separation for testability**: pure state module (`transcript.rs` or `state.rs`)
   holding a reducer `apply_event(&mut TranscriptState, &WebChannelEvent)` with no
   terminal deps — unit-testable. Rendering (`render.rs`) and event loop (`app.rs`)
   stay thin.

## Tests / verification (definition of done)

- Unit tests for the reducer: text_delta accumulation, thinking vs text separation,
  chat_done replaces with full_response, chat_error, ignores other client_ids,
  tool_call/result lines.
- `src/core/cli_tests.rs`: `tui_subcommand_reports_disabled_build_when_gate_off`
  (+ `chat` alias) under `#[cfg(not(feature = "tui"))]`, mirroring the mcp tests
  (assert error contains "tui feature disabled" and NOT "unknown namespace").
- Builds (Apple Silicon: prefix `GGML_NATIVE=OFF`):
  - `cargo check --manifest-path Cargo.toml`
  - `cargo check --no-default-features --features tokenjuice-treesitter` (disabled build)
  - `cargo test --lib core::cli` and the tui module tests, both feature directions:
    `cargo test --lib --no-default-features --features tokenjuice-treesitter core::`
- `node scripts/ci/check-feature-forwarding.mjs` passes with the allowlist entry.
- `cargo fmt` clean.

## Docs

- AGENTS.md: add `tui` row to the feature table + a short gate section
  (leaf-ish gate, sheds `ratatui`+`crossterm`, intentionally not forwarded to desktop).
- `src/openhuman/platform/about_app/`: add user-facing feature entry for the terminal chat UI.

## Non-goals (v1)

- No thread-picker UI beyond `--thread/--new` flags, no markdown rendering,
  no image/artifact display, no approval-request interaction (surface as a status
  line telling the user to handle it elsewhere), no remote-core (HTTP) mode.

## Workflow

Small focused commits on `feat/tui-chat` in this worktree; don't push.
Debug logging with `[tui]` prefix on all state transitions.
