# openhuman-tui

A conversation-first terminal frontend for OpenHuman, built with ratatui and
crossterm. It embeds the core through `openhuman_rpc::host::tui()` with no HTTP
server or sidecar. Login, saved conversations, model/agent selection, approvals,
tools, child-run inspection, and diagnostics use the existing core contracts.

## Build and view

From the repository root:

```bash
bash scripts/ci-cancel-aware.sh cargo build -p openhuman-tui
./target/debug/openhuman-tui --demo
```

`--demo` is an offline interactive preview using the production renderer and
pointer geometry with clearly labeled sample events. It does not initialize a
core, authenticate, write conversations, or call a model. Account and execution
actions in the preview explain that they need live mode.

For the real runtime and saved conversations:

```bash
./target/debug/openhuman-tui --resume
```

In an existing tmux session, open a new window at an unused index:

```bash
tmux new-window -t workspace:7 -n openhuman -c "$PWD" './target/debug/openhuman-tui --resume'
```

If using the user's dedicated Mosh server, add `-L mosh` to tmux commands.
Use a terminal of at least 80×24 for comfortable navigation. Wider screens use
the same compact header, transcript, composer, and status layout.

## Controls

| Action                | Keyboard               | Mouse                           |
| --------------------- | ---------------------- | ------------------------------- |
| Commands              | Ctrl+P or `/help`      | Commands button                 |
| Send / steer          | Enter                  | Send; Enter steers during a run |
| Queue follow-up       | Ctrl+Q during a run    | Queue                           |
| Newline               | Shift+Enter or Ctrl+J  | —                               |
| Focus controls        | Tab / Shift+Tab        | Hover/click                     |
| Stop                  | Esc from composer      | Stop                            |
| Scroll transcript     | Page Up / Page Down    | Wheel                           |
| Return to live tail   | Jump-to-latest control | Jump to latest                  |
| Expand tool/reasoning | Focus then Enter       | Click the row                   |
| Exit                  | Ctrl+D or `/quit`      | —                               |

Tab accepts command completion when a slash suggestion is open; otherwise it
moves focus. Ctrl+C closes an overlay, clears a draft, or requests cancellation,
depending on the focused state. It does not immediately quit. Pasting text never
sends it automatically. Closing the TUI shuts down its in-process runtime.

Terminal support for modified Enter, mouse forwarding, and native selection
varies. `--no-mouse` keeps native terminal mouse behavior; `/mouse` toggles
capture in a running TUI. System themes use ANSI colors and the terminal's
default background. Dark, Light, and Monochrome are selectable with `/themes`;
`NO_COLOR` starts in Monochrome. Theme choices apply to the current process.

## Login, agents, and activity

`/login` opens a provider picker (Google, GitHub, Twitter, Discord);
`/login google` starts that provider directly. Like Medulla, this opens the
TinyHumans browser flow with a nonce-protected, ephemeral loopback callback.
Credential exchange, profile validation, encrypted persistence, and account
adoption remain in `openhuman_tinyhumans::SessionManager`. The TUI never renders
tokens. Login and saved-account validation run in the background; the callback
expires after five minutes. Settings has mouse controls to reopen the browser,
copy its link via OSC52 (terminal support required), or cancel. Esc also cancels.
Already accepted callbacks finish their atomic session update.

Over SSH, forward the callback port shown in Settings before opening the copied
link locally: `ssh -L <port>:127.0.0.1:<port> <host>`. `/login-token` retains
masked one-time-token entry as a fallback. New sessions record the issuing
backend origin; changing backends cannot send that JWT to a different origin.
Restore the original backend or sign in again. Legacy sessions without origin
metadata remain compatible. Expired sessions clear the account display; account
changes discard prior-account drafts and create an account-owned conversation.
Product login is distinct from an inference provider's key configuration.

`/agents` loads runnable profiles and writes `config.update_agent_settings`'s
`chat_agent_id` after selection. This is the core's configured chat agent for
subsequent turns, not an isolated per-thread preference. `/model` reads the
current provider's catalog; `/model <id>` offers an explicit override. Catalog
requests run in the background; a dismissed or superseded view cannot be
replaced by a late response.

Tool results update their originating call. Calls retain bounded arguments and
output, timing, and status; `/tools` opens the stored preview. `/subagents`
opens read-only child output. Pending approvals and plan review have a persistent
attention control. Inspecting a child does not switch the parent conversation
or discard its draft. Failed/incomplete stored activity is labeled honestly;
unsettled historical activity is marked stale.

`/sessions` or `/resume` switches saved conversations while retaining each
draft. History failures keep the current conversation. The initial history page
contains up to 500 projected items. Full session branching, child steering/
cancellation, older-page loading, and complete uncapped output retrieval are
not implemented by this UI.

Additional commands: `/new`, `/rename`, `/delete`, `/permissions`, `/status`,
`/usage`, `/skills`, `/mcp`, `/artifacts`, `/diff`, `/review`, `/copy`, `/export`,
`/clear`, `/chat`, `/logs`, `/config`, `/settings`, `/logout`, `/quit`.
`/clear` hides the visible transcript without cancelling a run or deleting its
durable history. `/permissions` explicitly reports when the autonomy policy is
off; stored tiers have no enforcement effect until that policy is enabled.

## Rendering and input

The loop redraws only dirty state or active animation. Live paints are
coalesced at 16 ms. The bottom-anchored composer reflows on resize while the
transcript retains its scroll anchor. Settled blocks are wrapped once per content revision and width;
wide row offsets avoid 16-bit history overflow. Only viewport rows are copied
into the frame, and formatted blocks far from the viewport are evicted while
their height metadata and underlying transcript remain available.

Input has a bounded queue with backpressure. Rendered cell rectangles are the
source of pointer hit testing, including overlays. Drag/release events do not
activate controls. Tool/text content is sanitized before terminal rendering.
Large tool previews are bounded; display truncation never deletes core history.
Absolute performance budgets in the design spec are targets, not benchmark
claims about every terminal or platform.

`./target/debug/openhuman-tui --bench` measures the viewport and production
renderer offline. `python3 scripts/debug/tui-profile.py --mode live` measures
real terminal input-to-paint, idle CPU/output, and RSS in an isolated workspace
without signing in or sending a model prompt. See
[measured results and limits](../../docs/performance/tui-viewport.md).

## Source map

| Path under `src/`                   | Responsibility                                                      |
| ----------------------------------- | ------------------------------------------------------------------- |
| `runner.rs`, `main.rs`              | CLI parsing, host boot, offline preview entry                       |
| `app.rs`, `app/`                    | Event loop, runtime actions, pickers, sessions, file/Git operations |
| `actions.rs`, `input.rs`            | Shared actions and cell-based pointer translation                   |
| `render.rs`, `render/`              | Conversation, controls, diagnostics, and overlay rendering          |
| `viewport.rs`                       | Cached wrapping and viewport selection                              |
| `state.rs`, `activity.rs`           | Live/replayed transcript and tool/child projections                 |
| `composer.rs`, `ui_state.rs`        | Unicode editor, navigation, draft, and presentation state           |
| `effects.rs`                        | Background catalog results and stale-view protection                |
| `presentation.rs`, `theme.rs`       | Local commands, read-only inspection, semantic colors               |
| `controls.rs`, `session.rs`         | Safe configuration and host-owned account flow                      |
| `demo.rs`                           | Offline interactive visual preview                                  |
| `terminal.rs`, `crash_reporting.rs` | Terminal cleanup, panic restoration, crash reporting                |

Hosts depend only on `openhuman-rpc` and its curated facades. Harness execution,
tool policy, queueing, and durable session behavior remain with their owners.
No model/tool implementation is copied into the frontend.

## Validation

```bash
bash scripts/ci-cancel-aware.sh cargo test -p openhuman-tui
bash scripts/ci-cancel-aware.sh cargo clippy -p openhuman-tui --all-targets -- -D warnings
cargo fmt -p openhuman-tui --check
```

Tests cover transcript identity/deduplication, tool and child status, replay,
bounded file discovery, Unicode input/wrapping, masked login, overlay hit
precedence, stale catalog results, large history/cache bounds, and CLI parsing.
Live authentication and model execution still require a configured account or
provider. This frontend ships beside `openhuman-core` in the CLI packages.
