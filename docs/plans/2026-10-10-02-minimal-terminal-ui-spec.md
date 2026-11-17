# 2. OpenHuman minimal terminal UI specification

Status: initial terminal implementation delivered; remaining target behavior is recorded below. Date: 2026-10-10. Scope confirmed: terminal UI first; OpenClaw is the intended third harness. Evidence: [1. Harness UI research](2026-10-10-01-harness-ui-research.md). Code baseline: OpenHuman `f6e17bc343`.

## Product goal

An OpenCode-inspired terminal workspace where the user can sign in, issue commands, choose an agent, read responses, follow tools/subagents, and control everything with a keyboard or mouse. The UI should answer four questions without opening a panel: which conversation is open, which agent/model will act, what is happening, and whether the user needs to respond.

Deliver the seven requested capabilities: login, commands, views, agents, subagents, tool calls, and colors. Reading and inspection should consume most of the screen. The default screen has one transcript and one composer. Advanced capabilities remain available through commands. Desktop redesign is a later application of these interaction rules, not part of the first implementation.

## Layout

At 80×24: one header line, a transcript filling remaining space, a composer of 3–7 lines including controls, and a one-line footer. A pending decision gets a compact band above the composer. At 120×40 or wider, the user can explicitly open a 28-column activity inspector; it is closed by default. Below 80 columns, collapse optional metadata and render the inspector as an overlay. At very small sizes, prioritize draft text and the active decision, with a compact resize hint.

```text
 OpenHuman   UI cleanup ▾                 Sessions  Agents  Settings
 ──────────────────────────────────────────────────────────────────
 You
 Make the login view simpler.

 OpenHuman
 I’ll inspect the current flow and its tests.
   ✓ Read login.rs                                      120 ms  ▸
   ◌ Explore · locate login tests                        running ▸
   ! Shell · run focused tests                     needs approval ▸

 [Pending approval · Review]                       [Jump to latest]
 ┌────────────────────────────────────────────────────────────────┐
 │ Describe a task…                                               │
 │ Agent ▾  Model ▾                         Commands   Send / Stop │
 └────────────────────────────────────────────────────────────────┘
 Ready · Workspace: project        Local · Signed in    ? shortcuts
```

This is a proposed wireframe, not an observed competitor screenshot. Use content-fitting controls; hidden controls remain reachable through the palette. Show context usage in the footer when known and space permits; put exact cost/cache breakdown in `/usage`. Unknown usage is “unavailable,” never zero.

Keep borders on the composer and overlays; use separators and whitespace for the transcript. No repeated outer boxes around every response. Markdown headings, lists, code fences, links, and diffs have minimal terminal styling; ordinary messages retain full text. User and assistant labels are text, not large avatars. Code wraps only when requested; provide horizontal inspection or a dedicated code viewer.

## Views and navigation

| View         | Default content                            | Activation                      | State preserved                               |
| ------------ | ------------------------------------------ | ------------------------------- | --------------------------------------------- |
| Chat         | Messages and compact activity              | Launch, `/chat`                 | Draft, scroll anchor, expanded rows           |
| Sessions     | Searchable title/status/updated list       | Header, `/sessions`, `/resume`  | Parent draft; selection stable during refresh |
| Agents       | Available profiles and running children    | Header, `/agents`, `/subagents` | Current conversation and draft                |
| Tool details | Arguments, output, status, timing          | Click/Enter on row              | Transcript position                           |
| Settings     | Account, provider/model, appearance, input | Header, `/settings`             | Draft                                         |
| Diagnostics  | Core state and bounded logs                | `/status`, `/logs`              | Draft and selected session                    |
| Diff         | File list and patch inspection             | `/diff`                         | Selected file, scroll, parent view            |

Only Chat is a permanent content view initially. Overlay content loads on demand, with a loading/error/empty state and explicit retry. Closing an overlay returns focus to its initiating control. Switching sessions saves the old draft, loads the new session, then enables Send. Failed history loads preserve the selected session and draft; they do not fall back silently.

## Login and model access

Product-account login and inference-provider connection are distinct controls. The TUI already supports one-time login tokens through `SessionManager`; ship a polished version of that working flow first. Offer `/login`, the account control in Settings, and an inline Sign in action when managed inference requires it. Do not block local/provider-key work merely because the product account is signed out when the runtime supports it.

Login states: signed out → entering token → authenticating → signed in; invalid/expired token → input with actionable error; cancellation → signed out. Mask entry from the first character. Token entry never enters composer history, transcript, export, or logs. Clear/zeroize secret buffers on completion, cancellation, and account changes. Keep authentication requests asynchronous so the UI can redraw and cancel.

On identity change, discard account-bound caches/selection, refresh access/model availability, and start or load an identity-owned conversation through existing host APIs. Hide previous-user content immediately. Retain only non-secret presentation preferences where appropriate. Logout names the account and confirms the action.

Browser OAuth/device-code entry is a later enhancement only if a supported host flow exists. Do not invent an auth URL or reintroduce auth exchange into core. Provider choices use catalog data with availability reasons; do not use free-form model IDs as the only model selector.

## One action system

Define `ActionId` and action metadata once: label, description, aliases, applicable view/focus, availability reason, shortcut, and effect. Mouse clicks, key shortcuts, slash commands, and palette rows dispatch the same action. Disabled actions explain why. Pure navigation actions never send a model prompt.

| Command                         | Behavior                                           | Implementation status at baseline                           |
| ------------------------------- | -------------------------------------------------- | ----------------------------------------------------------- |
| `/help`                         | Search commands/shortcuts                          | Existing, improve                                           |
| `/login`, `/logout`             | Account flow                                       | Logout exists; login action connects existing Settings flow |
| `/new`                          | New conversation, preserve old draft/history       | Existing, improve                                           |
| `/sessions`, `/resume`          | Session picker                                     | Resume exists; add sessions alias                           |
| `/rename`, `/delete`            | Session action; delete requires named confirmation | Existing                                                    |
| `/model`, `/models`             | Catalog picker and effective selection             | Existing free-text override; improve                        |
| `/agents`                       | Browse/select primary profiles                     | Existing browsing only; add selection                       |
| `/subagents`                    | Children for current parent; inspect               | New view over existing event data                           |
| `/tools`, `/details`            | Current activity, disclosure preferences           | New presentation actions                                    |
| `/permissions`                  | Effective autonomy policy and permitted changes    | Existing; show enabled/disabled policy explicitly           |
| `/approvals`                    | Pending decision list                              | Existing                                                    |
| `/status`, `/usage`             | Runtime/session facts                              | Existing                                                    |
| `/settings`, `/config`, `/logs` | Open relevant controls                             | Existing; replace permanent tabs                            |
| `/themes`                       | System/light/dark/high-contrast choices            | New presentation action                                     |
| `/diff`, `/review`              | Inspect changes / request agent review             | Existing; preserve distinct effects                         |
| `/copy`, `/export`              | Copy answer / export transcript                    | Existing; disclose limitations/errors                       |
| `/skills`, `/mcp`, `/artifacts` | Existing discovery surfaces                        | Existing, retained in palette                               |
| `/chat`, `/quit`                | Return to Chat / exit                              | Chat alias new; quit existing                               |

Defer `/undo`, `/redo`, session branching, arbitrary shell prefixes, and full agent-profile editing until their persistence/policy contracts are verified. Do not register a command as functional when it only closes a menu. `/clear` must explicitly say whether it hides display or starts a new conversation; keep that separate from delete.

## Composer, focus, and queueing

Enter sends when idle. During an active run, Enter steers using the core's `steer` lane; the button reads Steer. A separate Queue action submits `followup`. Do not label an `interrupt` send as steering. Explicit Interrupt remains available in the palette if needed. Preserve rejected/failed queued input in the draft.

Tab navigates focus when no autocomplete is open. Within completion, Tab accepts the selected completion. Move the current Tab-to-queue behavior to an explicit Queue shortcut/action; describe the migration in help. Shift+Enter inserts a newline when supported; Ctrl+J is the dependable fallback. Paste never executes a slash command or sends without a separate confirmation keystroke. A pasted `/quit` is editable text until deliberately submitted.

Focus priority: secret/decision dialog → ordinary overlay → composer/menu → transcript. Esc closes the focused popup first. From the composer it cancels an active run; idle Esc leaves the draft intact. Stop requests cancellation and shows Stopping until the runtime confirms completion; do not report success immediately. Ctrl+C clears a nonempty draft, otherwise requests stop during a run, otherwise shows an exit hint; Ctrl+D or `/quit` exits. Exiting during work must make the in-process host shutdown effect clear.

Arrow keys edit the composer, except inside an active suggestion list. Up/Down history navigation applies only at the relevant boundary or with an explicit history action. Add `/` command suggestions and `@` file search without stealing cursor movement. File completion inserts a reference; any actual file-read/attachment behavior follows the existing core contract.

## Agents and children

Primary-agent chooser: name, description, mode/role, effective model, and access summary. Filter hidden/system-only definitions. Bind selection to the existing chat request's supported routing parameter, not merely a UI badge. Apply changes to the next turn; explain frozen session prompt/tool behavior where a new session is needed. Agent profile is independent from model and autonomy policy.

Children appear under the spawning activity, with task label, state, latest public progress, elapsed time, and tool count when known. Distinguish running, waiting for user, succeeded, failed, cancelled, and unknown/stale. Colors supplement text. Pin running/attention rows; completion does not suddenly change their position under the pointer.

Selecting a child opens an inspector with its available transcript/activity and a parent breadcrumb. Preserve the parent draft and viewport. Inspection is read-only unless the runtime exposes a steer/continue contract for that child. A dedicated worker thread and an ordinary child run are separate types. Show Stop only with a supported scoped cancel operation. Missing child history is “not available,” not an empty fabricated conversation.

Identity keys must include thread/request/task identity. `skill_id` and subagent detail currently carry child information; verify the exact mapping from `channel_event.rs` and the progress bridge. A repeated spawn/resume must update the same child or its new run generation according to the event contract. Never join concurrent children by display name.

## Tool calls and pending decisions

One persistent row per call, keyed by thread + request + tool-call ID. Row fields: tool display label, safe target detail, status marker/text, duration, and disclosure affordance. Track pending args → running → succeeded/failed/cancelled; awaiting approval or awaiting user is distinct. A result updates the originating row instead of appending a second disconnected row.

Arguments and output are expandable. Keep the summary to one or two lines; preview at most 20 lines or 4 KiB. Indicate truncation and whether a supported artifact/output-reference API can retrieve more. Do not promise complete output if the wire already capped it. Retain raw structured data for inspection only within a bounded cache; render JSON and code only when opened. Escape terminal control sequences in all model/tool/user text.

Pending approvals remain prominent even when details are collapsed. Show tool, sanitized command/target, reason, scope, and supported decisions. Default to no decision; never make Enter on a newly opened approval auto-approve. Preserve user input when a request arrives. Include expiry from authoritative metadata when available; expiry/cancellation removes decision buttons while retaining a record. Distinguish OAuth connection grants from ordinary execution approval.

Plan review and questions use the same attention system with their own actions. Opening details does not suspend or duplicate a runtime request. Pending state survives view changes; after event gaps, reconcile against authoritative pending APIs. Show errors with cause/recovery guidance from structured metadata, not regex-derived guesses.

## Mouse specification

| Target                | Pointer behavior                                       | Keyboard equivalent           |
| --------------------- | ------------------------------------------------------ | ----------------------------- |
| Header/pickers        | Click opens associated view                            | Palette/command               |
| Menu row              | Hover highlights; click selects/activates              | Up/Down + Enter               |
| Tool/child row        | Click disclosure or inspect                            | Focus + Enter/Space           |
| Transcript            | Wheel scrolls; manually scrolling disables follow      | PgUp/PgDn                     |
| Jump to latest        | Click restores following                               | End in transcript focus       |
| Composer              | Click sets Unicode-aware cursor                        | Arrow/Home/End                |
| Send/Steer/Queue/Stop | Click dispatches explicit action                       | Shown shortcut                |
| Decision              | Click a labeled button, then guarded decision handling | Focus + deliberate activation |
| Outside overlay       | Dismiss ordinary popup; preserve pending decision      | Esc                           |

Use a layout/hit-region snapshot shared by rendering and input. Dispatch only against the currently displayed layout; invalidate on resize. Coordinates are terminal cells, not string bytes. Account for scroll offsets, wide characters, combining marks, wrapped lines, and overlay precedence. Draw visible hover/focus feedback without a separate high-frequency timer.

Wheel events act on the panel under the pointer and are coalesced for trackpads. Ignore drag/release events for activation; one press causes one action. In tiny layouts, remove the corresponding hit target when a control is hidden. Support a `/settings` mouse-capture toggle and a proposed `--no-mouse` escape hatch for native terminal selection. Document terminal-specific selection/link modifiers; do not promise browser-style drag selection universally.

## Color and typography

Proposed dark palette: canvas `#111315`, raised `#1B1E22`, main text `#E7E9EC`, secondary text `#A4ABB5`, border `#3A424D`, accent `#79A8F2`, success `#7CC99A`, attention `#E7BD68`, error `#F18B8B`. Proposed light palette: canvas `#FAFAF9`, raised `#FFFFFF`, main text `#20242A`, secondary text `#59616E`, border `#CDD2D9`, accent `#245EB5`, success `#267746`, attention `#88620D`, error `#B52E3D`.

These are candidate tokens, not claims of measured contrast. Validate body/secondary text contrast at 4.5:1 and functional boundaries/focus at 3:1 where applicable. Use terminal defaults in System mode, with bounded capability detection and immediate ANSI fallback. Support truecolor, 256-color, 16-color, and monochrome rendering. Honor `NO_COLOR`. Never communicate failure, access level, or selected agent through hue alone.

An agent may have a small identity accent, but success/attention/error colors retain their meaning. Use plain terminal typography, mild emphasis, and spacing. Provide ASCII status/disclosure fallbacks for terminals with poor glyph support. Animation is one restrained activity indicator; support reduced-motion/static output.

## State and integration architecture

Keep the existing in-process `openhuman_rpc::host::tui()` host. The TUI depends on `openhuman-rpc` and its curated facades; no direct core dependency and no new HTTP service. Credentials stay in the host session owner. Harness execution, queueing, delegation, durable sessions, and tool policy remain in their owning core/vendor modules.

Proposed file responsibilities under `crates/openhuman-tui/src/`:

- `actions.rs`: typed actions and metadata; shared availability and dispatch routing.
- `layout.rs`: responsive rectangles and hit-region snapshot.
- `input.rs`: focus-aware key, mouse, paste, and resize translation.
- `theme.rs`: semantic colors and terminal fallbacks.
- `state.rs` plus focused activity modules: typed transcript/run/tool/child projections.
- `render.rs` plus `render/` modules: pure view rendering and cached block layouts.
- `app.rs`: async event orchestration, dirty tracking, effect completions, shutdown.
- Existing `composer.rs`, `cockpit.rs`, `controls.rs`, `session.rs`: evolve rather than duplicate.

All host effects complete through messages tagged with thread/identity/selection generation. Slow RPC/auth/history requests must not block the main input loop. Ignore stale effect results after account or session changes. Handle broadcast lag by marking activity stale and requesting supported transcript/pending-state reconciliation. Do not infer success from a dropped event.

Use `WebChannelEvent` fields for call IDs, sequence, display labels, timing, usage, and child details. Preserve event order and dedupe only events with a valid identity; legacy events need a documented fallback. Keep a monotonic receipt clock for provisional elapsed time, replaced by authoritative timing. Never synthesize cost or model progress.

## Performance requirements

Targets below are proposed acceptance budgets, not existing benchmark results. Measure on a recorded machine/terminal and separate render costs from harness startup and inference.

| Workload                                                  | Target                                                                |
| --------------------------------------------------------- | --------------------------------------------------------------------- |
| Idle, no live countdown or animation                      | No periodic redraws after initial paint                               |
| User action to next frame                                 | p95 ≤ 50 ms locally                                                   |
| Frame calculation at 120×40, 10,000 settled activity rows | p95 ≤ 16 ms after cache warmup                                        |
| Streaming text                                            | Coalesce paints at ≤ 30 fps; retain every delta and flush final state |
| Large log/tool payload                                    | Bounded preview; no formatting full payload on closed disclosure      |
| Hidden inspector/overlay                                  | No render, polling, or formatting work for its contents               |
| Transcript growth                                         | Paginated history and bounded hot caches; durable history retained    |

Cache layout by entry revision + width + theme/disclosure revision. Settled entries do not rewrap on every delta. Maintain cumulative row heights with a wide integer and locate the viewport by indexed search; render visible blocks plus a small overscan. Avoid the current full `Text` reconstruction and `u16` total-height accumulation. Anchor scroll by entry/line, so expanding a row or receiving text does not move the material being read. Following resumes only on explicit action or reaching the end.

Draw on dirty state and necessary animation ticks only. Coalesce redundant resize/mouse-move events while preserving key/paste/decision order. Bound queued events with an explicit overflow/recovery policy; do not silently drop completion or approval events. Evict offscreen formatting caches without deleting durable transcript data.

## Build and debug sequence

1. Baseline: capture 80×24/120×40 frames, measure idle redraws and large-transcript frames, run existing TUI tests. Record toolchain and fixture sizes.
2. Shell: make Chat primary; build semantic themes, responsive layout, focus/actions, palette, and settings navigation. Keep existing commands functional.
3. Input: mouse hit testing, wheel/follow behavior, cursor positioning, paste safety, explicit send/steer/queue/stop. Verify every key action has the intended mouse route.
4. Activity: structured call/child states and disclosures with deterministic fixtures, then live/replay integration. Add approval/question attention states without stealing input.
5. Account and selection: polished token login, catalog model picker, effective agent routing, and identity/session reset behavior.
6. Efficiency: incremental viewport rendering, deferred details, dirty redraws, paging, bounded caches; repeat the measured workloads.
7. Runtime debug: use the mock backend for a full login → select agent/model → send → tool → approval → child → cancel → resume scenario. Exercise a real PTY for mouse and resize. Record differences from reference screenshots.

Each milestone should be independently usable and reviewable. Test changed behavior and invariants, not decorative implementation details. Follow the repository's `*_tests.rs` convention; long builds/tests run through `scripts/ci-cancel-aware.sh`, with output under the checkout/shared configuration rather than temporary build directories. Coverage on changed behavior must meet the existing 80% diff gate.

Focused checks: `cargo test -p openhuman-tui`, `cargo check -p openhuman-tui`, `cargo fmt --check`, crate-chain/layout checks, and the necessary enabled/disabled feature forwarding checks if manifests change. Root compilation may be expensive; establish failures before calling the new UI ready. Do not turn off hooks/tests to achieve a pass.

## Acceptance cases

- Sign in/out with keyboard and mouse; invalid token retry; cancelling clears secrets; no token in export/history/logs.
- A command selected from slash/palette/mouse invokes exactly the same effect; unavailable actions explain why.
- Concurrent calls of the same tool remain separate; duplicate result updates do not produce duplicate rows.
- Inspect a running child and return with parent draft, expanded state, and viewport preserved.
- Approval arriving during composition does not submit/erase text or auto-approve; expiry disables stale controls.
- Enter steers, Queue follows up, Stop awaits confirmation; labels match actual core parameters.
- History read failures and event lag are visible; stale-session responses cannot overwrite a newer selection.
- Mouse coordinates remain correct after resize, scrolling, disclosure, and Unicode wrapping.
- Large history exceeds 65,535 rendered lines without overflow; idle drawing stops; bounded cache stays bounded.
- Dark/light/system/monochrome layouts remain legible; status can be understood without color.
- PTY smoke covers macOS, Linux, Windows Terminal, and tmux/Mosh forwarding; failures are documented with version and reproduction steps.

## Known implementation gates

Agent routing, model catalog shape, child transcript retrieval/cancellation, question response, and complete tool-output retrieval require contract verification before wiring controls. Where an existing API cannot support a proposed action, initially show a clear unavailable state and implement the capability in its owning repository as a separate scoped change. This spec does not authorize replacing the harness or copying vendored implementation into the TUI.

The initial implementation now covers the conversation shell, shared actions and mouse hit testing, session/model/agent pickers, masked token login, tool/child inspection, themes, cached viewport rendering, and offline preview. See [the TUI guide](../../crates/openhuman-tui/README.md) for launch commands and controls.

Verified locally: baseline tests, TUI unit/CLI tests, build, scoped Clippy, crate-chain/layout checks, and live/demo startup plus terminal mouse navigation in macOS tmux. The broader dependency Clippy command encounters a pre-existing `type_complexity` lint in the core relay store. Windows/Linux terminal behavior, real provider execution, and successful backend login were not exercised in this implementation session.

Follow-up implementation adds Medulla-style browser login through the shared TinyHumans session owner, background saved-session validation, issuing-backend binding for new sessions, mouse login controls, and a bottom-anchored responsive composer. The indexed viewport avoids full-history scans on settled paints and point edits. Offline benchmarks and isolated real-PTY profiling are documented in [the performance report](../performance/tui-viewport.md); these are development-profile observations on one machine, not cross-platform guarantees. Successful interactive provider sign-in is still an operator validation step.

Remaining extensions: device-code login, session branching/older-page loading, scoped child steering/cancellation, interactive question forms, persistent appearance preferences, and Markdown/diff-specific rich formatting. Current themes are process-local; child/output inspectors show stored bounded previews. The performance budgets above remain measurement targets; the implementation does not claim every budget has been met on every platform.
