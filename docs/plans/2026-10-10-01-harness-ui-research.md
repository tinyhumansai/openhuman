# 1. Harness UI research

Research date: 2026-10-10. Target: OpenHuman terminal UI first, confirmed by the user. The four references are OpenCode, Codex, OpenClaw, and Pi Agent. Companion specification: [2. OpenHuman terminal UI](2026-10-10-02-minimal-terminal-ui-spec.md).

## Evidence and limits

This is a documentation-led interaction audit, plus inspection of OpenHuman source at `f6e17bc343`. Competitor binaries were not installed or exercised. Layout recommendations are design proposals, not measured replicas. Mouse support is marked verified only where a source describes it. No competitor performance measurements are available; fewer controls alone does not prove lower CPU or memory use.

OpenCode currently publishes both V1 and V2 documentation. This audit uses V2 for the target interaction model; some V2 details are beta. Its TUI guide and agent guide disagree about agent cycling: the TUI/keybind reference says Shift+Tab, while the agent page mentions Tab and Shift+Tab. Treat the keybind reference as the current binding reference, and do not mix V1 commands or credential storage with V2. Pi's original `badlogic/pi-mono` URL redirects to `earendil-works/pi`; current primary docs are used below.

## OpenCode: principal interaction reference

### Conversation and composer

The documented loop is project → prompt → transcript → follow-up. `/` discovers commands; Ctrl+P opens the action palette. `@` attaches workspace files, including line ranges. `!` enters shell mode with output retained in the session. Enter during work steers; Ctrl+X then Enter queues. Session tabs, session search, recent projects, an external editor, and a separate side-question dialog support navigation without abandoning the current task. Undo/redo can change files as well as conversation history, so they are not merely display operations. [TUI guide](https://opencode.ai/v2/docs/cli/tui/)

### Agents and subagents

V2 distinguishes selectable primary agents from child-session subagents; a custom profile can support both. Build and Plan are primary profiles; General and Explore are subagents. Hidden maintenance agents are excluded from the picker. Profiles combine prompts, model preferences, permissions, and display metadata. An `@` mention can request delegation. A child uses its configured permissions, which must not be mistaken for inherited read-only access. [Agent guide](https://dev.opencode.ai/v2/docs/agents/)

### Login and connection

`/connect` offers provider-specific authentication methods: keys, OAuth, commands, and environment credentials. The TUI can open or copy authorization details. Saved provider accounts can be selected and managed separately from a session's model. V2 stores saved credentials in its server database; environment credentials have a different lifecycle. This is provider connection, not the same thing as OpenHuman product-account login. [Provider connection](https://opencode.ai/v2/docs/cli/providers/)

### Views, activity, and mouse

The keybind reference exposes sidebar and terminal-pane toggles, a session timeline, child/parent navigation, tool exploration grouping, and a diff viewer. It explicitly documents clicking files/folders and wheel scrolling in the diff tree, right-click review actions, and clicking a running shell command to inspect captured output. These examples verify mouse interaction in those views; they do not establish every control's pointer behavior. Keyboard bindings are configurable by command ID, and disabled bindings are supported. [Keybind reference](https://opencode.ai/v2/docs/cli/keybinds/)

### Colors

Light, dark, and detected system modes are available. Themes distinguish text, surfaces, interaction, and feedback rather than assigning unrelated colors to each widget. The current V2 documentation notes that native V2 custom-theme loading is unfinished and legacy files are migrated. Adopt the semantic-token idea, not the beta file format. [Themes](https://dev.opencode.ai/v2/docs/themes/)

### Secondary web surface

The documented web UI is served by the same server as the TUI. Pairing links establish browser/app sessions; this is a distinct connection layer from provider authentication. Its existence is useful evidence for shared backend semantics across interfaces, not a reason to replace OpenHuman's in-process TUI with a server dependency. [Web access](https://opencode.ai/v2/docs/cli/web/)

Design interpretation: make the transcript dominant; put choices in searchable overlays; reveal activity incrementally; preserve session context when inspecting children. OpenCode is the primary reference for the proposed OpenHuman interaction vocabulary.

## Codex: control and inspection reference

### Terminal surface

The official CLI page shows a small startup summary with model and working directory, a prompt, and context remaining. First use includes sign-in; local work stays in a terminal conversation. The documented workflow includes inspecting commands and diffs as they appear and steering the active turn. The startup box is reference evidence, not a specification for every later screen. [Codex CLI](https://learn.chatgpt.com/docs/codex/cli)

### Commands and history

The slash menu filters commands in the composer. Relevant controls include model, permissions, plan mode, status, diff, resume/fork, compact, copy, and agent-thread selection. The current reference distinguishes Enter steering from Tab queueing during work and includes file references and prompt-history search. Shell commands use the active approval/sandbox settings. `/agent` and `/subagents` select an agent thread; they are not menus for creating a new agent profile. [Developer commands](https://learn.chatgpt.com/docs/developer-commands?surface=cli)

### Subagent visibility

Official guidance describes separate inspectable agent threads, with the main thread collecting results. CLI users can switch through `/agent`; supported desktop/IDE surfaces expose child activity. A useful product distinction is the profile definition versus the currently running child thread. Additional agents incur additional model/tool work. [Subagents](https://learn.chatgpt.com/docs/agent-configuration/subagents)

### Desktop observations and uncertainty

The current official features page includes an illustrated project/recents navigation surface with composer additions. The old Codex app/features URL now redirects into ChatGPT documentation. Consequently this audit does not claim pixel-accurate parity with a particular Codex desktop release. Terminal mouse behavior, complete tool disclosure rules, and exact theme tokens were not established by the inspected official pages. [Current features surface](https://learn.chatgpt.com/docs/features)

Design interpretation: keep active model, access policy, run state, and child identity legible; expose commands and diffs without permanent navigation clutter. Use clear labels for steering, queueing, stopping, and starting a new conversation.

## OpenClaw: session and connection reference

### TUI

OpenClaw documents both Gateway-connected and embedded local modes. The header identifies the connection, agent, and session; the transcript contains messages, notices, and tool cards. The footer exposes run/model/usage state. Agent, model, and session pickers are separate. Ctrl+O controls tool expansion; Ctrl+T controls thinking visibility. Pending questions can be collapsed and reopened, and reconnect restores them in Gateway mode. External channel delivery is separate from sending to the harness and is fixed at launch. Local provider auth has its own command. [TUI](https://docs.openclaw.ai/web/tui)

### Children

A child run has a lifecycle, while its session can persist. The documented Control UI presents Running/Finished lists from the parent; selecting a child opens a view-only transcript beside it and keeps the parent's draft. Persistent visible sessions are a separate concept and can be steered. Do not imply that every ordinary subagent is an editable independent chat. [Subagents](https://docs.openclaw.ai/tools/subagents)

### Web control surface and efficiency

The Control UI documentation explicitly discusses deferred panel initialization, shared metadata requests, refresh-on-change, stable open lists, and pausing background narration when hidden. These are architectural techniques worth carrying over. They are not measured proof that OpenClaw is faster than another harness. [Control UI](https://docs.openclaw.ai/web/control-ui)

Design interpretation: preserve the exact agent/session association, show stale or unavailable data honestly, and keep questions visible without forcing a modal over unfinished input. Connection errors must never silently select a different conversation.

## Pi Agent: density and extension reference

### Deliberately small core

Pi describes itself as an extensible harness with interactive, automated, RPC, and SDK entry points. It explicitly leaves built-in subagents and plan mode out of its core; extensions/packages can provide them. Pi therefore cannot be treated as evidence of a built-in multi-agent management UI. [Pi README](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/README.md)

### Conversation and controls

The transcript includes prompts, responses, tools, results, and errors; the editor and footer carry composition and model/context/cost state. Tool output and thinking are independently collapsible. Enter can guide ongoing work; Alt+Enter adds follow-up work. Slash menus expose login, model, settings, and session operations. Session tree/fork/clone provide history exploration. Fullscreen and normal-scrollback modes are distinct. [Terminal usage](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/usage.md)

### Colors and terminal limitations

Current Pi themes include system, light, and dark. System mode derives colors from terminal responses with fallbacks and contrast adjustment. [Theme guide](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/themes.md)

The setup guide documents modified-Enter differences, IME positioning, fullscreen wheel behavior, and terminal-native link modifiers. Mouse capture can interfere with native selection/link affordances; terminal and multiplexer behavior needs direct testing. [Terminal setup](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/docs/terminal-setup.md)

Design interpretation: spend screen space on the transcript and a compact footer, keep advanced controls discoverable, and make terminal limitations explicit in help.

## Cross-harness decisions

These are OpenHuman design decisions inferred from the sources above, not additional competitor feature claims.

| Area          | Proposed OpenHuman decision                                 | Main reference                 |
| ------------- | ----------------------------------------------------------- | ------------------------------ |
| Main view     | Conversation first; compact header/composer/footer          | OpenCode, Pi                   |
| Login         | Dedicated masked account flow; separate provider connection | OpenCode, current OpenHuman    |
| Commands      | One action registry for slash, palette, keys, and mouse     | OpenCode                       |
| Views         | Sessions/Agents/Settings overlays; logs as diagnostics      | OpenCode, Pi                   |
| Primary agent | Select profile; expose effective model and policy           | OpenCode                       |
| Subagents     | Compact task rows; inspect child while preserving parent    | Codex, OpenClaw                |
| Tool calls    | One expandable row per call; state and duration visible     | OpenCode, OpenClaw             |
| Questions     | Pending indicator survives dismissal and navigation         | OpenClaw                       |
| Colors        | Neutral surfaces, semantic accent/status colors             | OpenCode, Pi                   |
| Efficiency    | Incremental rendering; bounded previews; deferred panels    | OpenClaw docs, OpenHuman audit |

## OpenHuman source audit at f6e17bc343

| Finding                                                           | Source                                                          | Implication                                                        |
| ----------------------------------------------------------------- | --------------------------------------------------------------- | ------------------------------------------------------------------ |
| Standalone ratatui frontend over `openhuman-rpc`                  | `crates/openhuman-tui/Cargo.toml`, `README.md`, `src/runner.rs` | Extend this crate; retain the in-process host                      |
| Four top-level tabs: Logs, Chat, Config, Settings                 | `src/ui_state.rs`, `src/render.rs`                              | Make Chat primary; move infrequent controls behind overlays        |
| Captures mouse; loop ignores mouse events                         | `src/terminal.rs`, `src/app.rs::run`                            | Mouse control requires hit testing and action dispatch             |
| Full transcript projection/wrapping during each draw              | `src/render.rs::draw_transcript`                                | Cache settled blocks and render the visible range                  |
| Unconditional 120 ms ticker; loop draws before every select       | `src/app.rs::run`                                               | Add dirty tracking and only animate when needed                    |
| Flat `Entry { kind, text }`; separate text rows for call/result   | `src/state.rs`                                                  | Retain stable call IDs and structured states                       |
| Children reduced to descriptive tool rows                         | `src/state.rs::apply_event`                                     | Retain child lifecycle and parent links                            |
| Agents overlay closes on Enter without selecting a profile        | `src/app.rs::handle_overlay_key`                                | Agent browsing is not agent selection yet                          |
| Enter uses steer during a run; Tab queues                         | `src/app.rs::send_or_command`, `src/app.rs::handle_tab_key`     | Preserve steering; give queue its own action so Tab can move focus |
| Login is one-time-token entry owned by SessionManager             | `src/session.rs`, `src/controls.rs`                             | Reuse the host auth owner; browser login is a separate enhancement |
| Rich web-channel call IDs, timing, labels, sequence, child detail | `crates/openhuman-core/src/web_chat/channel_event.rs`           | Consume existing contracts before proposing backend work           |
| Desktop already has rich disclosures and child state              | `app/src/store/chatRuntimeSlice.ts`, `AssistantUiToolCall.tsx`  | Reference semantics; avoid inventing competing states              |

## Research still needed during implementation

1. Exercise a pinned OpenCode release for actual transcript spacing, click targets, selection, and child navigation; record its version and screenshots.
2. Run OpenHuman in a PTY at 80×24 and 120×40 to establish visual and performance baselines.
3. Verify live versus replay events for child transcripts, tool arguments/output, pending questions, and run recovery; do not promise unexposed actions.
4. Verify terminal capabilities on macOS terminals, Windows Terminal, Linux, and the user's tmux/Mosh environment. Modified keys and mouse forwarding vary.
5. Benchmark before/after rendering and event reduction separately from inference/network latency.

No UI implementation or runtime validation is claimed by this research document.
