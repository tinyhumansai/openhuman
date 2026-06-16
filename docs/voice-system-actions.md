# Voice → System Action Feature Tracker

**GitHub Issue:** [#3148](https://github.com/tinyhumansai/openhuman/issues/3148)  
**Branch:** `feat/voice-always-on-all` (cumulative) — **all merged to `main`.** Phase 1 landed via [#3168](https://github.com/tinyhumansai/openhuman/pull/3168); the full feature was split from the mega-PR [#3307](https://github.com/tinyhumansai/openhuman/pull/3307) (now **closed** in favour of the stack) into an 8-PR stack — **all 8 merged 2026-06-04**: [#3340](https://github.com/tinyhumansai/openhuman/pull/3340) (main-thread input + CEF fix), [#3341](https://github.com/tinyhumansai/openhuman/pull/3341) (AX/UIA perception + automate engine), [#3342](https://github.com/tinyhumansai/openhuman/pull/3342) (wire automate/ax_interact tools), [#3343](https://github.com/tinyhumansai/openhuman/pull/3343) (Phase 2 always-on engine + RPC), [#3344](https://github.com/tinyhumansai/openhuman/pull/3344) (always-on Settings toggle + i18n), [#3345](https://github.com/tinyhumansai/openhuman/pull/3345) (notch status pill — supersedes the closed [#3166](https://github.com/tinyhumansai/openhuman/pull/3166)), [#3346](https://github.com/tinyhumansai/openhuman/pull/3346) (Phase 3 fast command router), [#3362](https://github.com/tinyhumansai/openhuman/pull/3362) (Phase 1.5 vision-click fallback)  
**Started:** 2026-06-02  
**Last updated:** 2026-06-09 — added **Change 1.17** (PR [#3558](https://github.com/tinyhumansai/openhuman/pull/3558)): browser & app (Spotify/Apple Music/Slack) keyboard-shortcut fast-paths, cross-platform shortcut tables, the `hotkey` verb, artist-aware Music verification, and actionable `automate` failure responses — all from live-transcript analysis. Earlier this day: verified all 8 always-on stack PRs merged to `main` (code paths confirmed post-refactor [#3424](https://github.com/tinyhumansai/openhuman/pull/3424)); global push-to-talk ([#3090](https://github.com/tinyhumansai/openhuman/issues/3090) via [#3349](https://github.com/tinyhumansai/openhuman/pull/3349)) landed separately. **Only open item across all phases: the on-device audio wake-word model (Phase 3) — not started; text-based "Hey Tiny" match remains the interim.**  

---

## Goal

Enable the app to continuously listen to the user, understand spoken commands, and perform system actions on the laptop — e.g., saying *"open my Music player"* causes the app to open it, without any hotkey press or manual send.

---

## Companion Feature (Separate PR)

**Notch Live Activity Indicator** — [PR #3166](https://github.com/tinyhumansai/openhuman/pull/3166)  
A transparent NSPanel pill at the top of the primary screen (the macOS notch area) that shows live voice/agent status. Built alongside this feature; will light up automatically once always-on listening is implemented.

---

## Phase 1 — Quick Wins ✅ Complete

> Low-effort changes that make the existing hotkey-triggered dictation flow work end-to-end without manual sends or approval prompts.

---

### Change 1.1 — Auto-send after transcription

**Status:** ✅ Done  
**Commit:** `7269f4373`

**Problem:** After speaking via the dictation hotkey, the transcript appeared in the chat composer but the user had to press Enter manually to send it.

**Fix:**
- `app/src/hooks/useDictationHotkey.ts` — added `autoSend: true` to the `dictation://insert-text` event detail
- `app/src/pages/Conversations.tsx` — `onDictationInsert` now checks the flag; when set, calls `handleSendMessage(text)` directly instead of inserting into the textarea. Added `handleSendMessageRef` (updated every render) so the mount-time effect can access the latest send function

**Result:** Press hotkey → speak → message auto-sends to agent. No Enter key needed.

---

### Change 1.2 — Shell allowlist for app-launching — ⚠️ REVERTED / SUPERSEDED

**Status:** ❌ Reverted — superseded by Change 1.4 (`launch_app`) and the security review.

**What was tried (commit `7269f4373`):** added `"open"` / `"xdg-open"` to `READ_ONLY_BASES` so `open -a Music` would run without an approval prompt.

**Why reverted:** base-command classification can't see args, and `open`/`xdg-open`/`start` can open arbitrary `https://` URLs and custom URI handlers — too broad for the `Read` (no-approval) path (maintainer security review). They were **removed** from `READ_ONLY_BASES`; the current code (`policy_command.rs:514-520`) deliberately keeps them out, with a comment. App launching now goes through the dedicated, gated `launch_app` tool (Change 1.4), which is scoped to named applications only.

---

### Change 1.3 — Shell tool description fix

**Status:** ✅ Done  
**Commit:** `ec8f5be2e`

**Problem:** Shell tool description said *"Execute a shell command in the workspace directory"* — the LLM was reasoning that it could only run workspace commands, not launch apps.

**Fix:**
- `src/openhuman/tools/impl/system/shell.rs` — updated description to explicitly mention system actions and app launching examples

**Result:** Agent now understands the shell tool can perform system actions, not just workspace file operations.

---

### Change 1.4 — Dedicated `launch_app` tool

**Status:** ✅ Done  
**Commit:** `802fbca76`

**Problem:** Using the `shell` tool for app launching requires loosening `workspace_only` and expanding `allowed_commands` — a security regression. The `shell` tool also couldn't be used because the orchestrator's strict `named` tool list excluded it.

**Fix (production approach):**
- `src/openhuman/tools/impl/system/launch_app.rs` — **new tool** with `PermissionLevel::ReadOnly` (never triggers approval gate)
  - macOS: `open -a "<app_name>"` via `tokio::process::Command`
  - Linux: `gtk-launch`, fallback `xdg-open`  
  - Windows: `Start-Process` via PowerShell
  - Input validation: rejects paths, metacharacters, empty names
  - Unit tests: name, permission, schema, validation, error cases
- `src/openhuman/tools/impl/system/mod.rs` — registered module + pub use
- `src/openhuman/tools/ops.rs` — added `LaunchAppTool` to `all_tools_with_runtime`
- `src/openhuman/tools/user_filter.rs` — added `"launch_app"` family, `default_enabled = true`
- `app/src/utils/toolDefinitions.ts` — added to frontend tool catalog (Settings → Agent Access toggle)

**Result:** Agent has a purpose-built, always-allow tool for launching apps. No shell exposure, no path security concerns.

---

### Change 1.5 — Orchestrator agent tool scope

**Status:** ✅ Done  
**Commit:** `7d04fc4bc`

**Problem:** Even though `launch_app` was registered, it was invisible to the agent. The orchestrator (`src/openhuman/agent_registry/agents/orchestrator/agent.toml`) has a strict `named = [...]` allowlist. `launch_app` was not in it, so it was filtered out. Confirmed via logs: `visible=24, names=[...no launch_app...]`.

**Fix:**
- `src/openhuman/agent_registry/agents/orchestrator/agent.toml` — added `"launch_app"` to the `[tools] named` list, alongside `"current_time"` (same pattern: direct answer without delegation)

**Confirmed working via logs:**
```text
visible=25, names=[..., launch_app, ...]
[launch_app] ▶ execute called  app_name="Music"
[launch_app] macOS: running `open -a "Music"`
[launch_app] macOS: `open -a` exit=exit status: 0  stderr=
[launch_app] ✓ launch succeeded  msg="Opened 'Music'."
```

**Result:** Saying "open my Music app" now opens Music directly. No approval prompt, no delegation, no refusal.

---

### Change 1.6 — SOUL.md capability hint

**Status:** ✅ Done  
**Commit:** `cdd3bb4a4`

**Problem:** Even with the tool available, the agent was refusing ("I can't open apps on your device") because its training overrides the function-calling schema.

**Fix:**
- `src/openhuman/agent/prompts/SOUL.md` — added explicit *"What you can do on the user's machine"* section listing `launch_app`, `shell`, `file_read`/`file_write` with the instruction: *"Never say 'I can't open apps' when you have a tool to do it. Use the tool."*

**Result:** Agent now knows it has these capabilities and is instructed to use them.

---

### Change 1.7 — Diagnostic logging

**Status:** ✅ Done  
**Commit:** `cdd3bb4a4`

**Added logging to:**
- `src/openhuman/tools/impl/system/launch_app.rs` — logs every step: `▶ execute`, validation pass/fail, platform dispatch, `open -a` exit code + stderr, fallback result
- `src/openhuman/agent/harness/session/builder.rs` — logs the **full list** of visible tool names at session build time (previously only logged count)

**Result:** Can now confirm at a glance whether `launch_app` is in the tool list and trace every step of its execution.

---

---

### Change 1.8 — Computer control (mouse + keyboard) — ⚠️ REVERTED

**Status:** ❌ Reverted (commits `50ca434b7` add, `bi0rd96sa` revert)

**Problem:** Agent could open apps but couldn't interact with their UI.

**What was tried:** Enabled the existing `mouse` + `keyboard` tools (enigo / `CGEventPost`), wired into the orchestrator, user filter, and frontend catalog.

**Why reverted:** `CGEventPost` injects synthetic events to the **currently focused window**. When the focused window was OpenHuman's own CEF renderer (the chat UI), a Space keypress crashed the app — `EXC_BREAKPOINT / SIGTRAP` in `CFRelease → NSKeyValueWillChangeWithPerThreadPendingNotifications → -[NSApplication stop:]`. CEF can't handle arbitrary key injection. Confirmed via crash report `OpenHuman-2026-06-02-035139.ips`.

**Replaced by:** Change 1.9 (`ax_interact`) — AXUIElement targets elements directly by label with no synthetic events and no CEF crash risk.

---

### Change 1.9 — AXUIElement app UI interaction (`ax_interact`)

**Status:** ✅ Done  
**Commits:** `4f9ca1cad` (feature), `2c32b59c9` (exact-match fix), `betuerj11`/test commits

**Problem:** Need to interact with desktop app UIs reliably, without the CEF crash from synthetic events.

**Fix — uses the macOS Accessibility API (AXUIElement) instead of CGEventPost:**
- `src/openhuman/accessibility/helper.rs` — extended the unified Swift helper with three commands:
  - `ax_list` → walk the AX tree, return interactive elements (buttons, fields, cells)
  - `ax_press` → `AXUIElementPerformAction(kAXPressAction)` by label, **exact match preferred over contains** (so "Play" beats "Playlist")
  - `ax_set_value` → `AXUIElementSetAttributeValue(kAXValueAttribute)` by label
- `src/openhuman/accessibility/ax_interact.rs` (new) — Rust wrappers `ax_list_elements`, `ax_press_element`, `ax_set_field_value`
- `src/openhuman/tools/impl/computer/ax_interact.rs` (new) — `AxInteractTool` with actions `list` / `press` / `set_value`, `PermissionLevel::ReadOnly`
- `src/openhuman/accessibility/ax_interact_tests.rs` (new) — integration tests (open Music → search AC/DC → find row → press)
- Wired into `tools/ops.rs`, `tools/user_filter.rs`, `toolDefinitions.ts` (App UI Control), `orchestrator/agent.toml`, `SOUL.md`

**Why it's better than mouse/keyboard:**

| | mouse/keyboard (reverted) | ax_interact |
|---|---|---|
| Mechanism | `CGEventPost` synthetic events | `AXUIElementPerformAction` direct API |
| CEF crash risk | Yes | None |
| Coordinates | Required (needs screenshot) | None — finds by label |
| Works when app unfocused | No | Yes |

**Verified working:** Direct AX test against Music listed 256 elements including `Bollywood Hits`, `Play`, etc.; pressing `Bollywood Hits` then `Play` both returned `exact=true` and acted correctly.

---

### Change 1.10 — Multi-step UI workflow guidance

**Status:** ✅ Done

**Problem:** When asked to "play Highway to Hell by AC/DC", the agent ran: launch → list → press Library → press Songs → press "Show Filter Field" → set_value "Highway to Hell" → **press "Play"**. The final press hit the **global playback bar Play button** (plays last queue item), not the specific song row. Result: app navigated correctly but the wrong/no track played.

**Fix:**
- `src/openhuman/agent/prompts/SOUL.md` — added explicit multi-step workflow:
  1. `list` → discover elements
  2. `set_value` → type in filter/search
  3. `list` **again** → see filtered results
  4. `press` the **specific item** (song row), not the generic Play button
- Added Apple Music guidance: use `shell` to open `music://music.apple.com/search?term=...`, then `ax_interact list` to see song rows as AXCells, then press the specific row. More reliable than the Library filter field.

**Result:** Agent is directed to select the specific item before pressing playback, instead of pressing the global Play button after filtering.

---

### Change 1.11 — Apple Music two-step play (navigate then play)

**Status:** ✅ Done

**Problem:** When asked to "play Highway to Hell by AC/DC", the agent navigated to the right screen but **nothing played**. Pressing a search-result row in Apple Music only *selects/navigates* — it does not start playback. The agent then pressed the global transport Play button, but nothing was queued.

**Investigation (empirical AX probing against live Music):**
- Every "Highway to Hell" element (AXCell, AXGroup, AXButton) exposes only the `AXPress` action — which selects/navigates, never plays.
- Double `AXPress`, a real CGEvent double-click on the Top-Results card, and AX-select + Return key **all left player state `stopped`**.
- **Working sequence found:** AXPress the search-result card to **navigate into the song's detail page**, then AXPress the **Play button on that detail page** → `player state: playing` ✅

**Fix:**
- `src/openhuman/agent/prompts/SOUL.md` — replaced the Apple Music guidance with the exact 5-step sequence: URL-scheme search → list → press song row (navigates in) → list detail page → press detail-page Play. Explicitly warns that pressing a search result only navigates, and the second Play press is mandatory.
- `src/openhuman/accessibility/ax_interact_tests.rs` — `test_full_flow_search_and_play_acdc` exercises the full navigate-then-play flow and **logs** the `osascript ... get player state` outcome. Playback is **best-effort, not hard-asserted** (Apple Music's UI is nondeterministic — see change 1.13); the test hard-asserts only the tool-level press/list successes.

**Verified:**
```text
[step 4] navigate into song: Ok("Pressed 'Highway to Hell' in 'Music'.")
[step 5] press detail Play: Ok("Pressed 'Play' in 'Music'.")
[step 6] player state: playing
test ... ok
```

---

### Change 1.12 — One-shot `play_music` tool (root-cause fix)

**Status:** ✅ Done

**Problem:** Even after change 1.11, the agent still used the broken filter-field approach and didn't play. Transcript analysis (`~/.openhuman/users/<id>/workspace/session_raw/*.jsonl`) revealed two real root causes:

1. **The orchestrator has no `shell` tool.** Change 1.11 put the play guidance in `SOUL.md` — but the orchestrator runs with `omit_identity = true` and **never sees SOUL.md**. Change 1.11b moved it to the `ax_interact` description, which told the agent to "use the shell tool to open `music://...`" — but the orchestrator can't run shell (it delegates). The agent wrapped the command in a `prompt` arg to a delegation tool; it never executed, and it fell back to the filter approach.
2. **Cross-chat memory contamination.** The user message was prefixed with `[Cross-chat context — historical]` containing prior filter-approach "Progress Checkpoint" steps, biasing the agent back to the wrong method.

**Fix — stop relying on the LLM to orchestrate a fragile multi-step flow with a tool it lacks. Encapsulate the whole proven sequence in native Rust:**
- `src/openhuman/accessibility/ax_interact.rs` — `play_apple_music(query)`: open search URL → AX-find + press song cell (navigate) → press detail-page Play → verify `player state == playing`
- `src/openhuman/tools/impl/computer/play_music.rs` (new) — `PlayMusicTool`, single call `play_music{query}`, `PermissionLevel::ReadOnly`, runs the blocking flow via `spawn_blocking`
- Registered in `ops.rs`, `user_filter.rs`, `orchestrator/agent.toml`, `toolDefinitions.ts`

**Result:** Agent calls `play_music{query:'Highway to Hell AC/DC'}` **once**; Rust does search→navigate→play. No shell dependency, no multi-step LLM orchestration, no filter-field fallback. Unit tests pass; the underlying flow is exercised by `test_full_flow_search_and_play_acdc` (tool-level success hard-asserted, playback best-effort). **Note:** `play_music` was later removed in change 1.13 in favour of the generic `ax_interact` tool — this entry documents the investigation that led there.

**Key learning:** The orchestrator (chat agent) only reads **tool descriptions + agent.toml** — NOT SOUL.md (omit_identity=true). Behavior guidance for the chat agent must live in tool descriptions or be encapsulated in the tool itself.

---

### Change 1.13 — Generic any-app tool + filtered list (remove play_music)

**Status:** ✅ Done

**Problem:** "Play Numb by Linkin Park" still failed, and the agent **hallucinated**. Transcript (`session_raw/*.jsonl`) showed:
1. `play_music` hit a 4s timing race — results hadn't rendered, so it returned "No matching song found. Top result cells: [empty]".
2. The agent fell back to `ax_interact list`, which dumped **273 elements**. The tool result was **truncated mid-list**, so the model reasoned over a partial view and hallucinated a wrong result ("Numb - Single by Marshmello").

**Feedback:** A music-specific tool is the wrong abstraction. Build a generic tool that interacts with **any** app.

**Fix:**
- **Removed** `play_music` tool + `play_apple_music` helper and all registrations.
- **`ax_interact` is now a robust generic any-app tool:**
  - `ax_list_elements_filtered(app, filter)` — Rust-side label filter so `list` returns only relevant elements (fixes the truncation→hallucination root cause).
  - `list` action takes a new `filter` param; output capped at 60 elements with a "narrow your filter" hint; empty-match returns a "UI may still be loading" hint instead of failing hard.
  - Description rewritten to be app-agnostic and document the general **navigate-then-activate** pattern (pressing a list row/search result selects/opens it; press the action button afterward) — no hardcoded Apple Music steps.

**Key learning:** Dumping a full AX tree (hundreds of elements) overflows the tool-result budget; the truncated view makes the model hallucinate. Always filter list results to keep them small and accurate.

---

### Change 1.14 — `automate(app, goal)`: Rust-driven multi-step automation ✅ M1–M5 done

**Status:** ✅ **M1 + M2 + M3 + M4 + M5 shipped; M3 proven live on macOS.** The only remaining `automate` work is the **vision fallback** for Electron/partial-AX apps — tracked under **Phase 1.5** below and [`voice-automate-plan.md`](voice-automate-plan.md). (Per-app native fast-paths beyond the shipped Music one are **descoped**.)

**Agent-in-the-loop fixes (2026-06-03, from two live chat sessions):**
- **Mutations were off** — the agent correctly called `automate` but it (and `ax_interact`) refused because `computer_control.ax_interact_mutations=false`. Enabled it; also rewrote both refusal messages to point at **Settings → Agent Access** instead of a config key (the agent had relayed "controls are locked down").
- **Query mis-parse** — orchestrator goal `…search for "Highway to Hell" by AC/DC, and play it` made the after-"play" parser extract `"it"`. `extract_play_query` now prefers a **quoted title + `by <artist>`** and rejects bare pronouns. (Unit-tested with the exact failing goal.)
- **General loop spun** — pressed "Search" 11× to budget exhaustion. Added a **no-progress guard**: 3 identical actions in a row → abort.
- **Search-results timing** — the fast-path's retry burned out before catalog results rendered (`settle` reports count-stable while the network fetch is pending). Added a real, mockable `wait` between attempts (6 × ~800ms).

**M5 finding — AXEnabled is unreliable:** plumbed an `enabled` field end-to-end (Swift `axEnabled` → `AXElement.enabled` → Windows stub), but Apple Music reports its **pressable** search-result rows as `enabled=Some(false)`. Gating `pick_row` on it broke playback. So `enabled` is kept **informational only** (documented on the struct); matchers never skip on it. The better future actionability signal is AXPress-action support, not AXEnabled.

**M4 — live progress in the notch (2026-06-03):** the notch indicator (originally PR #3166) was cherry-picked onto this branch (`feat(notch)` + fmt commits → `notch_window.rs` NSPanel + `notch/NotchApp.tsx`, auto-shown on startup, transparent when idle). The `automate` loop and Music fast-path now call `overlay::publish_attention(...)` at each step (`Opening …`, `Searching Music for …`, `Pressing …`, `Typing …`, `Playing …`, plus done/fail), which the existing Socket.IO bridge emits as `overlay:attention` and the notch renders as a pill — so the user sees the automation happening live. Verified: app boots with `[notch-window] panel shown at top-center`; Tauri shell + frontend compile; 31 automate unit tests green.

**M3 live proof (2026-06-03):** `music_fastpath_live` drives real Apple Music end-to-end and **hard-asserts `player state == playing`** — confirmed: pre-state `paused` → post-state `playing`. Three bugs the live runs surfaced, all fixed + tested:
1. **Perceive filter was the whole multi-word query** — a substring filter can't match a full title → now filters by the first strong token and `pick_row` does the token match.
2. **Search results render late (§1.13 race)** — retry perceive across up to 4 settles; `AC/DC` now percent-encodes correctly.
3. **False success: pressed the toolbar Play, not the song's** — the first run reported success but *nothing played*. AX probing showed the search screen has only the toolbar transport Play (empty queue → silence); pressing the song row navigates to a detail page where a **second** Play appears (23→24 controls). Fix: capture the baseline Play count, **wait for the detail Play to render**, press it, then **verify real playback** via `osascript … player state` and retry (≤3×). Added `verify_playing()` to `AutomateBackend` (macOS osascript; `None` elsewhere = best-effort). `automate` now only reports a play success when audio is actually playing — the false-success class (§1.11) is closed.

**M3 — shipped (Music fast-path):**
- `src/openhuman/accessibility/app_fastpaths/{mod.rs,music.rs}` (new) — deterministic accelerators consulted by `run()` **before** the general loop. Music encodes the §1.11 proven sequence: launch → open `music://…/search?term=…` → settle → press the song row (navigate) → settle → press the detail-page **Play**. Pure helpers `matches` / `extract_play_query` (handles "play X by Y", "launch Music and play …", "play X in Apple Music").
- **Structurally different from the removed `play_music` tool (§1.13):** this is *internal* to `automate`, not a tool the LLM selects, and on any failure/`None` the loop **falls through** to the general model-driven path — so it can only help. Added `open_url` to the `AutomateBackend` trait (cross-platform opener; fast-path only).
- **Tests:** 9 unit (parser cases, full scripted sequence, no-row fallthrough, dispatch) + 1 `#[ignore]` macOS live test. **Live proof on a Mac:** `cargo test --lib music_fastpath_live -- --ignored --nocapture` (needs Music + Accessibility permission).

**M2 — shipped (real settle):**
- `src/openhuman/accessibility/ax_interact.rs` — new `ax_wait_settled(app, stable_ms, timeout_ms)`: polls the app's interactive-element count and returns once it holds steady for `stable_ms` (or `timeout_ms` elapses). Portable — rides on `ax_list_elements`, which already cfg-dispatches (macOS AX / Windows UIA). Pure decision core `counts_settled(history, n)` extracted and unit-tested (5 non-OS-gated tests).
- `automate.rs` — `RealBackend::settle` now calls `ax_wait_settled` (240ms stable / 2s cap) via `spawn_blocking` instead of the M1 blind 450ms wait. This is the piece that removes the timing-race failure class (§1.11/§1.13): the next perceive always sees a settled tree. An AXObserver-driven settle can later sit behind the same signature.

**M1 — shipped:**
- `src/openhuman/accessibility/automate.rs` (new) — the perceive→decide→act→settle loop, generic over an injectable `AutomateBackend` (so the model + AX + launcher are all mockable). Strict JSON action schema (`launch`/`list`/`press`/`set_value`/`done`/`fail`) with a one-shot repair retry on unparseable output (never acts on a hallucinated guess), a step budget (default 12), and a snapshot cap (40 elements) mirroring `ax_interact`'s anti-truncation guard. `RealBackend` calls the existing AX primitives + `launch_platform`, and routes decisions through the **fast tier** (`create_chat_provider("memory", …)` for now; a dedicated `automation_provider` knob is a follow-up). Settle is a short fixed wait in M1 (M2 makes it AXObserver-driven).
- `src/openhuman/tools/impl/computer/automate.rs` (new) — `AutomateTool { app, goal }`. Always `Dangerous` + `external_effect` (routes through the ApprovalGate); reuses `ax_interact`'s mutations opt-in (`computer_control.ax_interact_mutations`) and the shared `is_sensitive_app` denylist.
- Registered everywhere: `tools/ops.rs`, `tools/user_filter.rs` (`automate` family), `orchestrator/agent.toml` (`named`), `app/src/utils/toolDefinitions.ts` (Settings → "App Automation").
- **Tests:** 18 passing — loop happy path, navigate-then-activate, app override, budget exhaustion, repair retry (1 ok / 2 fail), explicit fail, non-fatal press failure, JSON parse (plain/fenced/garbage), snapshot cap/empty-hint; tool gating (missing args, mutations-off, sensitive-app refusal, schema).

**Problem (the real-time bar):** The user's target is *"whatever I say happens, live, in front of me"* — e.g. *"Launch Music and play Numb by Linkin Park"* or *"open Slack and message Steven 'hi'"*. Today every UI step (`list` → `set_value` → `list` → `press` …) is a **separate chat-LLM turn**. A Slack message is ~7 turns; at 1–3 s each that's 15–25 s, and each turn is a fresh chance to hit a timing race (1.13) or hallucinate. The heavy chat model is sitting *inside* the click loop — the wrong place for it.

**Root causes (all four documented earlier in Phase 1):**
1. **Timing races** — `list`/`press` do a single AX walk with no settle/wait; the UI hasn't rendered yet (1.11/1.13).
2. **Navigate-then-activate is re-reasoned every call** — pressing a row selects; you must then press the action control. That logic lives in prose, so it's re-derived (often wrongly) each turn (1.10/1.11).
3. **Round-trip explosion** — N full chat turns per task = latency + cost + N chances to fail.
4. **Weak element model + no verification** — `list` returns flat `[role, label]`; `press` reports success on `AXAction == .success` even when nothing changed.

**Design — take the chat model out of the click loop:**
- **New tool `automate { app, goal }`** — one call from the orchestrator. Rust then runs a tight **perceive → act → verify** loop internally: read a *filtered* AX snapshot → pick the next action → act → **wait for the UI to settle (AXObserver, not fixed sleeps)** → verify it took effect → repeat until the goal is met or a step budget is hit.
- **A fast model drives the inner loop** (Haiku-class) with a *tiny* context: just the goal, the current small AX snapshot, and the last result — not the whole conversation. Each inner step is ~0.5–1 s and self-corrects, instead of one 3 s chat turn that falsely reports success.
- **Settle + verify in Rust** between steps — deterministic, kills the timing-race class in one place.
- **Native fast-paths for high-value apps** (skip the UI entirely where possible):
  - **Music** — `music://` search URL → AX play (already explored in 1.11), or AppleScript for library. **(Shipped — M3.)**
  - ~~**Spotify** — Web API search → `spotify:track:…` URI + AppleScript `play`.~~ **Descoped** — the general loop covers it; not worth the per-app maintenance now.
  - ~~**Slack** — deep link `slack://channel?…` to open the DM, then AX to type + send.~~ **Descoped** — handled by the general loop / vision fallback.
  The general AX loop is the fallback for everything else.
- **Vision fallback for Electron/Chromium apps** (Slack, Discord, VS Code, Spotify-desktop) whose AX/UIA tree is partial (documented limitation). Slack needs accessibility enabled (`defaults write com.tinyspeck.slackmacgap AccessibilityEnabled -bool true`, relaunch). Where AX returns empty, fall back to **screenshot → vision-locate → guarded click**. This is the reverted CGEventPost path (1.8) — but it crashed only when events hit *OpenHuman's own focused CEF window*; a guarded click into a *different, foregrounded* app does not have that failure mode.
- **Stream progress events** to the UI / notch pill (PR #3166) so the user sees each step happen live.

**Why a generic `automate`, not per-app tools:** Change 1.13 already established that app-specific tools (`play_music`) are the wrong abstraction. The abstraction that *is* generic is the **navigate-then-activate sequence itself** — `automate(app, goal)` encapsulates it once, in Rust, for every app, instead of asking the chat model to re-orchestrate fragile primitives every time.

---

## Phase 1.5 — Reliable, real-time multi-step automation ✅ Complete

> The bridge between today's `ax_interact` primitives and the always-on voice work. **Prerequisite for Phase 3** — fast voice routing into a slow/fragile action loop still feels slow. This is where "whatever I say happens, live" actually gets delivered.
>
> **Status:** the Rust inner loop (M1), poll-until-stable settle (M2), Music fast-path (M3, proven live), notch progress streaming (M4), the richer element model (M5), the **vision fallback for Electron/partial-AX apps** (Change 1.16), and **Change 1.17** (browser & app fast-paths + keyboard-shortcut driving + artist-aware Music + actionable responses) are all shipped. Per-app fast-paths now cover **Music, browsers, Spotify, Apple Music, and Slack** (Spotify/Slack — previously descoped — are now handled by the `app_shortcuts.rs` keyboard-shortcut path).

**Detailed implementation plan:** [`voice-automate-plan.md`](voice-automate-plan.md) — decided approach: **Rust inner loop + fast model**, first proof target **Music**.

**Planned files:**
- `src/openhuman/accessibility/automate.rs` (new) — the perceive→act→verify loop + settle/verify primitives, reusing `ax_interact` helpers.
- `src/openhuman/accessibility/app_fastpaths/` (new) — per-app deterministic paths behind a generic dispatch. **Shipped:** `music.rs` (M3). Further per-app fast-paths (Spotify/Slack) are **descoped** — the general loop handles them.
- `src/openhuman/tools/impl/computer/automate.rs` (new) — `AutomateTool { app, goal }`, gated like `ax_interact` (mutations opt-in, sensitive-app denylist reused).
- macOS helper (`accessibility/helper.rs`) — AXObserver-based settle (`ax_wait_settled`) + post-action verify; richer element model (enabled/onscreen/actions).
- Vision fallback — screenshot via `accessibility/capture.rs` → locate → guarded click (only when AX tree is empty, target app foregrounded, never OpenHuman's own window).

**Acceptance criteria:**
- [x] One `automate{app, goal}` call performs a multi-step flow end-to-end (no per-step chat turns)
- [x] Settle/verify removes the timing-race + false-success failure classes (1.11/1.13 do not recur)
- [x] Music flow ("play <song>") works end-to-end via the inner loop (proven live, hard-asserts `player state == playing`)
- [x] Electron/partial-AX apps fall back to vision+guarded-click without the CEF crash (Change 1.16)
- [x] Step-by-step progress streamed to the UI / notch indicator (M4)

---

### Change 1.15 — Full computer control (mouse/keyboard/screenshot) ✅ Crash fixed (main-thread dispatch)

**Status:** ✅ Keyboard/mouse now run on the app main thread → no CEF crash. Screenshot downscales for inline view. Live: `[computer] registered main-thread synthetic-input executor` on boot.

**The fix:** the crash was enigo's `TSMGetInputSourceProperty` running on a tokio worker (`_dispatch_assert_queue_fail`/SIGTRAP). macOS TSM must run on the main thread. New `tools/impl/computer/main_thread.rs` (`MainThreadInputOp` + `run_input_on_main`) dispatches each enigo op over the native registry to a handler the Tauri shell registers at startup, which runs it via `AppHandle::run_on_main_thread`. Keyboard + mouse tools no longer `spawn_blocking` enigo on a worker. Headless/CLI (no executor) returns a clear error instead of crashing. 66 keyboard/mouse tests green.

**Goal:** make the agent fully autonomous — when the accessibility tree is empty (Electron apps: Slack/Discord/VS Code), fall back to vision + synthetic input. Enabled `computer_control.enabled`, added `mouse`/`keyboard`/`screenshot` to the orchestrator `named` list + `autonomy.auto_approve`, and taught `prompt.md` a keyboard-first ladder (foreground via `launch_app` → `keyboard type` + Enter; Slack `Cmd+K` recipe).

**Foreground-first:** `automate::run` now `open -a`s the target app at the very start, always, so AX/input hit the right window.

**Screenshot fix:** oversized Retina captures were returned as "too large to base64-encode inline" (the model was blind). Now downscaled to a viewable JPEG (`downscale_to_jpeg`) with reported dimensions.

**Root cause (now fixed) — `OpenHuman-2026-06-03-170058.ips`:** `EXC_BREAKPOINT/SIGTRAP` on a **`tokio-rt-worker`** thread:
```text
enigo::macos::get_layoutdependent_keycode → TSMGetInputSourceProperty
→ dispatch_assert_queue → _dispatch_assert_queue_fail → SIGTRAP
```
enigo's keyboard-layout lookup (`TSMGetInputSourceProperty`) **must run on the app's main thread**; the keyboard tool ran on a tokio worker → macOS trapped. **Not** a focus issue (same §1.8 root cause); a frontmost-app guard would not have fixed it.

**Fix applied:** all enigo operations now run on the Tauri **main thread** via `run_input_on_main` (new `main_thread.rs` module + a native-registry handler the shell registers, dispatched through `AppHandle::run_on_main_thread`) and wrapped in `catch_unwind`. Keyboard/mouse tools **and** `vision_click` now execute without TSM traps.

**Tests:** voice-actions + autonomy suite is exhaustive — 220 feature unit tests + a JSON-RPC E2E (`json_rpc_voice_server_settings_roundtrip_always_on_and_wake_word`). The E2E caught + fixed real gaps (`wake_word` missing from the get output and the update RPC path). Screenshot downscale unit-tested.

---

### Change 1.16 — `vision_click`: vision fallback for Electron/partial-AX apps ✅ Done

**Status:** ✅ Shipped — closes the last open Phase 1.5 item. Electron/Chromium apps (Slack, Discord, VS Code) expose little or no AX/UIA tree, so the `automate` perceive→press loop had nothing to act on. The loop can now *see* the screen and click a described element.

**Problem:** when `perceive` returns an empty (or target-missing) element list, the loop was stuck — there was no label to press. The planned answer (tracker §1.5) was *screenshot → vision-locate → guarded click*, but two things blocked it: (1) the fast inner-loop model is text-only, and (2) the deferred **F2** coordinate-mapping gap — the screenshot is windowed + downscaled and Retina is 2×, while `mouse` expects absolute screen points, so a vision-returned coordinate would click the wrong spot.

**Fix — a new model-chosen `vision_click { description }` action in the `automate` loop:**
- **Reuses the existing multimodal path** — the OpenAI-compatible provider promotes an embedded `[IMAGE:<data-uri>]` marker into a real `image_url` part (`inference/provider/compatible_types.rs`, #3205), so vision-locate rides the existing `chat_with_system` call with **no new inference API**. The locate call uses the **main `chat`/vision provider** (`create_chat_provider("chat", …)`) — reliable UI grounding, and the fallback only fires when AX is empty (rare).
- **Folds in the F2 coordinate transform** — `src/openhuman/accessibility/vision_click.rs` (new): `CaptureGeometry` (window screen-rect in points + image pixel size) + a **pure, unit-tested** `image_to_screen(geom, px, py)`. The px→pt ratio absorbs both the downscale and the Retina backing scale, so no explicit scale factor is needed; the result is clamped strictly inside the target window.
- **§1.8 safety guard** — a `vision_click` only fires when the target app is **frontmost** (`focus::foreground_context`); on positive evidence a different app is focused it re-foregrounds once and otherwise **refuses**, so synthetic input never lands on OpenHuman's own CEF window. `None` (can't tell) = best-effort, since the loop already foregrounded the app at start.
- **Main-thread click** — the guarded left-click runs via `run_input_on_main` (Change 1.15), so off-thread enigo never traps TSM.
- Wired into the `automate` loop: new `Action.description`, a `vision_click` system-prompt verb ("use when the element list is EMPTY — Electron apps"), the no-progress signature extended with `description`, and `RealBackend::{screenshot,locate,frontmost_app,click}`. Inherits `automate`'s existing gating (Dangerous + mutations opt-in + sensitive-app denylist) — no new tool, no new approval surface.

**Tests:** pure `image_to_screen` (downscale / Retina 2× / origin offset / out-of-range + negative clamp / zero-dim safety), `parse_locate_response` (found / not-found / fenced / prose / garbage), `build_locate_user` (marker), `image_dims_from_data_uri` (real PNG round-trip); loop integration via the scripted backend (locate-and-click when frontmost, proceed when frontmost unknown, **refuse when another app is frontmost**, no-click on not-found, empty-description skip). A macOS live run against a real Electron app is the remaining manual validation (vision is nondeterministic — assert tool-level success, treat the visual outcome as best-effort, same caveat as Music).

---

### Change 1.17 — browser & app fast-paths, keyboard-shortcut driving, artist-aware Music, actionable responses ✅ Done

**Status:** ✅ Shipped (PR [#3558](https://github.com/tinyhumansai/openhuman/pull/3558)). One body of work driven by **live-transcript analysis of the `automate` agent** across browsers, Apple Music, Spotify, and Slack. The transcripts surfaced four recurring failures the agent kept hitting:

1. **Browsers** — for *"open my Brave browser, go to youtube.com and play a music video"* the `desktop_control_agent` was re-delegated **6×** (each sub re-launched Brave + re-navigated), ran `ax_interact` on the **wrong app names** (`Google Chrome`/`Safari`, never the real `Brave Browser`, because Chromium exposes no AX tree), then "played" via a **blind hardcoded mouse click at (400,350)** with no verification.
2. **Wrong track** — *"play Numb by Linkin Park"* played a **same-titled song by the wrong artist** ("Numb" by Marshmello & Khalid) and the agent **claimed success**, because the AX row label is title-only and verification only checked *that* something played.
3. **Inert responses** — the `automate` tool's replies were *honest but gave no next move*, so the agent **re-ran the same failing search 6×** instead of changing tactics.
4. **No keyboard-shortcut lever** — the loop hunted AX labels even where an app's own shortcut would be instant and reliable.

The six parts below address all four. All fast-paths are consulted by `automate::run` **before** the model loop (`try_fastpath` order: `music` → `browser` → `app_shortcuts`); on any failure they fall through, so they can only *help*.

**(a) Browser fast-path** — `app_fastpaths/browser.rs` (new). Pure parsers + `run(app, goal, backend)`:
  - `resolve_browser(app, goal)` — aliases → macOS display names (`brave`→`Brave Browser`, `chrome`→`Google Chrome`, `edge`→`Microsoft Edge`, `safari`/`firefox`/`arc`). Checks the `app` arg first, then the goal; a generic "browser" with no named product → **`None`** so it never guesses the wrong app (the original bug).
  - `extract_destination(goal)` — YouTube → `youtube.com/results?search_query=…` (percent-encoded); Google/"search for X" → `google.com/search?q=…`; a bare domain/URL → normalized to `https://…`.
  - **Navigation is one deterministic step:** `backend.open_url_in_app(browser, url)` → `open -a "<browser>" "<url>"` launches/foregrounds + navigates — no address-bar typing, no AX. Pure-navigation goals **complete here**; **play goals** navigate to the results URL then return non-success so the loop does the single "click first result" via `vision_click` (no native shortcut *selects* a result — we don't fake it). In-page media control ("pause/next the video") sends the YouTube shortcut directly.

**(b) Cross-platform browser shortcut table** — `app_fastpaths/browser_shortcuts.rs` (new). `shortcut(intent, browser, os)` resolves browser-level intents (address bar, new/close/reopen tab, tab N, next/prev tab, find, reload/hard-reload, back/forward, history, downloads, zoom, private window, fullscreen) per **Chrome/Firefox/Safari/Edge × macOS/Windows/Linux**. Nearly uniform (only ⌘↔Ctrl differs); encodes the real exceptions (Firefox-Linux `Alt+1‑8`, Firefox private window `⌘/Ctrl+Shift+P`, per-browser History/Downloads, Mac `⌘[`/`]` back-forward, `F11` vs `⌃⌘F` fullscreen). Sourced from the official Chrome/Firefox/Edge/Safari docs. `browser.rs` parses commands ("open a new tab", "go back", "reload", "switch to tab 3") and dispatches the resolved chord. (Brave/Arc/Chromium → Chrome family.)

**(c) App-shortcut fast-path** — `app_fastpaths/app_shortcuts.rs` (new). Same model for **Spotify, Apple Music, Slack**: `shortcut(intent, app, os)` + goal parser. Drives transport/navigation by each app's own global shortcut (faster + more reliable than AX). Media intents (play/pause, next, prev, volume ±, mute, shuffle, repeat, search) for Spotify + Apple Music; Slack nav (quick-switcher, search, new message/compose DM, next/prev unread, next/prev channel, threads, all-unread, mark read). Encodes the quirks: **Spotify uses ↓/↑ for next/prev** (not ←/→); **Apple Music prefixes everything with Ctrl on Windows** (Ctrl+Space vs bare Space on Mac); Slack channel-nav uses **Option/Alt+arrows**. Intents with no simple chord (Spotify volume, Apple Music mute / Windows search access-key) return `None` → fall through. Complementary to `music.rs`, which still owns Apple Music *song search/play*.

**(d) Keyboard-shortcut plumbing** —
  - Extracted shared `pub(crate)` helpers `run_hotkey`/`run_key`/`run_type_text` from `tools/impl/computer/keyboard.rs` (one place for validation + main-thread enigo dispatch, Change 1.15) — no behavior change to the `keyboard` tool.
  - New `AutomateBackend` methods (non-breaking defaults): `open_url_in_app`, `key`, `type_text`, `now_playing`. `RealBackend` impls `open -a` per-app, delegates keystrokes to the keyboard helpers, and reads Music state via osascript.
  - New **`hotkey` verb** in the general loop + system prompt (`{"action":"hotkey","keys":["Cmd","L"]}`) so the model can use any app's shortcuts — folded into the no-progress signature.

**(e) Artist-aware Music verification** — `music.rs` + `now_playing`. Apple Music's AX row label is title-only ("Numb - Single"), so a title match lands on the wrong artist. `music.rs` now parses the requested artist (`extract_artist`, "…by Linkin Park") and after Play reads the **now-playing name + artist** via AppleScript (`now_playing` → `osascript … get {name, artist} of current track`). On a match the summary names the verified track + artist; on a **mismatch it says so honestly** ("Now playing 'Numb' by 'Tom Odell'…") instead of claiming the requested track. Closes the wrong-artist false-success. **Follow-up:** *correcting* to the right artist is limited by AX `press`-by-label hitting only the first same-titled row — a library AppleScript `play (track whose name … and artist …)` fallback is the next step.

**(f) Actionable `automate` responses** — the tool's failures are now agent-actionable, not inert. Four fixes: **(1)** terminal failures (`music.rs` no-match/mismatch, `automate.rs` budget-exhausted/no-progress) carry **next-step guidance + a "don't repeat this" steer**; **(2)** they **list the actual candidates / on-screen labels** (`candidate_labels`, `screen_hint`) instead of a bare negative; **(3)** **verified-only success** — never "Playing 'X'" unless `now_playing` confirms it, and an artist/album press is reported as "navigated, no track started"; **(4)** `pick_row` now **prefers real song rows** (`AXCell`/`AXRow`) over artist/album buttons (the live bug pressed the "LINKIN PARK" artist row) and the step log names the element *type* pressed.

**Also:** the always-on toggle in `VoicePanel` is now shown in dev/debug builds (`IS_DEV_LIKE`) and hidden in production.

**Follow-up fix — Full OS access now enables app control.** `ax_interact` (press/set_value) and `automate` gated **only** on `computer_control.ax_interact_mutations`, which is independent of the autonomy level. A user who granted **Full** access in Settings → Agent Access still got *"App control isn't enabled yet…"* because that flag was untouched (confirmed live: `SecurityPolicy autonomy=Full` while the tool refused). Fixed with a shared `app_control_enabled(explicit_opt_in)` gate (`tools/impl/computer/ax_interact.rs`) that returns true when the explicit flag is set **or** the **live** policy (`security::live_policy::current()`) is `AutonomyLevel::Full` — so granting Full access takes effect immediately, no session restart or separate flag flip. Both refusal messages now point at Settings → Agent Access ("Grant Full access (or turn on App UI Control / App Automation)"). Covered by `app_control_enabled_combines_optin_and_full_access` + the two pinned-policy refusal tests.

**Follow-up fix — Windows browser nav + the re-delegation loop.** Live Windows transcript of *"open my browser, go to youtube.com and play a video"*: the orchestrator delegated to `desktop_control_agent` **7×**, every sub re-`launch_app`-ed Chrome (→ ~10 windows), and the AX path dead-ended — Chromium exposes no page content to UIA, `screenshot`/`vision_click` is unimplemented on Windows, and `ax_interact set_value` on the address bar **can't be submitted** (UIA Invoke on an Edit fails, no Enter). Two fixes:
> - **`open_url_in_app` now works on Windows** (`accessibility/automate.rs`) — was macOS-only (fell back to the *default* browser). Maps the resolved browser display name → the shell `start` App-Paths token (`windows_browser_launch_token`: Chrome→`chrome`, Edge→`msedge`, Brave→`brave`, Firefox→`firefox`; Safari/Arc→`None`→default handler) and runs `cmd /C start "" <token> "<url>"`. Launches/foregrounds the named browser and navigates in **one** step; when the browser is already open this lands in a **new tab of the existing window**, so the fast-path no longer piles up windows. The browser fast-path's deterministic nav is now cross-platform (mac `open -a`, win `start`). Tested by `browser_display_names_map_to_start_tokens` (`#[cfg(target_os="windows")]`).
> - **`desktop_control_agent` prompt steered to `automate` for browsers** (`agent_registry/agents/desktop_control_agent/prompt.md`) — it had `automate` but its only examples were Music/Slack, so it AX-fumbled the browser. Now: web browsers → use `automate{app, goal}` (deterministic URL nav), never type a URL into the address bar via `ax_interact`; **foreground each app at most once per task** (repeated `launch_app` piles up windows); and **stop after two failed attempts at a step** instead of re-launching and looping. (Still a Windows gap: `vision_click` for the final "click first result" — `screenshot` is unimplemented on Windows, so play-from-search-results can't complete deterministically yet; navigation/search now do.)

**Net for the browser example:** `open -a "Brave Browser" "https://www.youtube.com/results?search_query=music%20video"` + one `vision_click` — instead of 6 delegations and 5 failed AX calls.

**Tests (all green):** `app_fastpaths` **58**, `accessibility::automate` **21**, `computer::keyboard` **26**. Coverage: browser/app resolution + destination/intent parsers (incl. Spotify ↓/↑, Apple Music Win-Ctrl, Slack Alt-nav, generic-browser→None, percent-encoding); scripted-backend sequences (model-free nav, play→fall-through, media-control hotkey, dispatch order with music-wins-play); artist match→names-track / mismatch→honest; response shapes (candidate listing, wrong-artist steer, non-song honesty, song-row preference, budget/no-progress on-screen hints); a `hotkey`-verb loop test; `#[ignore]` macOS live tests. **Follow-ups:** wire the shortcut tables into the Phase 3 voice `command_router` (voice "pause"/"next" → frontmost-app shortcut); AppleScript library play-by-name+artist correction; expose `vision_click` as a standalone tool.

---

## Windows port — app interaction 🪟 ✅ Implemented

Phase 1's app-interaction layer is now ported to Windows. The macOS path uses the
Accessibility API via a Swift helper; the Windows path uses **Microsoft UI
Automation (UIA)** called directly from Rust (no helper process). The
agent-facing tool is a single `ax_interact` tool on both platforms — only the
backend differs, via cfg-dispatch. The sections below were the design plan; see
**"Windows port — implementation status"** at the end of this part for what
shipped and the test evidence.

### What already works cross-platform

| Capability | macOS (done) | Windows status |
|---|---|---|
| Auto-send dictation transcript | TS (`useDictationHotkey`/`Conversations`) | ✅ Already cross-platform (frontend) |
| App launching | `launch_app` / `policy_command.rs` | ✅ Launchers (`open`/`xdg-open`/`start`) stay OUT of `READ_ONLY_BASES` (can open arbitrary URLs/handlers); app launching goes through the gated `launch_app` tool. |
| `launch_app` | `open -a` | ⚠️ Already has a Windows branch (`Start-Process`) — verify it resolves app display names |
| `ax_interact` (list/press/set_value) | AXUIElement Swift helper | ❌ Needs a UI Automation (UIA) backend |

### 1. Launching apps on Windows (`launch_app`)

`launch_app.rs` already has a `#[cfg(target_os = "windows")]` branch using PowerShell `Start-Process "<app_name>"`. Caveats to verify on the Windows machine:
- `Start-Process "Spotify"` works for apps on PATH or registered App Paths, but **Store/UWP apps** (e.g. the Windows "Media Player", "Spotify" from the Store) need their AUMID: `Start-Process "shell:AppsFolder\<AUMID>"`. Consider enumerating Store apps via `Get-StartApps` (returns Name + AppID) and matching by display name.
- For URIs (e.g. `spotify:`, `mailto:`), `Start-Process "<uri>"` works the same as macOS `open`.

### 2. App UI interaction (`ax_interact` → UI Automation)

**The Windows analog of macOS AXUIElement is Microsoft UI Automation (UIA)** — the OS-level accessibility tree. It exposes the same concepts:

| macOS AX concept | Windows UIA equivalent |
|---|---|
| `AXUIElement` | `IUIAutomationElement` |
| `kAXRoleAttribute` (AXButton, AXCell…) | `ControlType` (Button, ListItem, Edit, Text…) |
| `kAXTitleAttribute` / `kAXDescriptionAttribute` | `Name` property (+ `AutomationId`, `HelpText`) |
| `AXUIElementPerformAction(kAXPressAction)` | `InvokePattern.Invoke()` (buttons) / `SelectionItemPattern.Select()` (list rows) |
| `AXUIElementSetAttributeValue(kAXValueAttribute)` | `ValuePattern.SetValue()` (text fields) |
| `AXUIElementCopyAttributeValue(kAXChildrenAttribute)` | `TreeWalker` / `FindAll(TreeScope.Descendants, …)` |
| Walk tree from app PID | `IUIAutomation.ElementFromHandle(hwnd)` or root + `ProcessId` condition |

**Recommended implementation path (Rust-native, no helper process needed):**
- Use the [`uiautomation`](https://crates.io/crates/uiautomation) crate (safe Rust bindings over the UIA COM API). This is cleaner than macOS, where we had to shell out to a Swift helper — on Windows the COM API is callable directly from Rust.
- Mirror the existing `accessibility::ax_interact` surface so the **tool stays identical**:
  - `list(app, filter)` → `CreateTreeWalker` over the app's window, collect elements whose `Name` matches `filter`, return `[{control_type, name}]`.
  - `press(app, label)` → find element by `Name` (exact-first), then call `InvokePattern` if supported, else `SelectionItemPattern.Select()`, else `LegacyIAccessiblePattern.DoDefaultAction()`.
  - `set_value(app, label, value)` → find `Edit`/`ComboBox`, call `ValuePattern.SetValue()`.
- **Key win over macOS:** UIA Invoke is generally a real "activate" (it triggers the control's default action), so the navigate-then-activate two-step that plagued Apple Music is less likely. A list-item Invoke on most Windows media apps plays directly. Still expect per-app quirks.

**Suggested module layout (parallel to macOS):**
```text
src/openhuman/accessibility/
  ax_interact.rs          # macOS (existing)
  uia_interact.rs         # NEW — Windows UIA backend, same fn signatures
  mod.rs                  # cfg-dispatch: pub use the right backend per-OS
```
Then `tools/impl/computer/ax_interact.rs` calls a thin cfg-gated facade so the **agent-facing tool is one tool on both platforms** (same name `ax_interact`, same actions). Update its description to be OS-neutral ("uses the platform accessibility API").

### 3. Permissions

- macOS needs the Accessibility permission. **Windows UIA needs no special permission** for same-session, same-integrity-level apps — a big simplification. Caveat: a non-elevated process can't drive an **elevated** app's UI (UIPI). If the agent must control an elevated app, OpenHuman would need to run elevated too (avoid unless necessary).

### 4. Diagnostics

Keep the same `[ax_interact]`/`[uia_interact]` log prefixes and the verbose step logging (`▶ action`, found-count, press result) — they were essential for diagnosing the macOS flow and will be just as useful on Windows.

### 5. Testing

Port the integration tests using a built-in Windows app that's always present and UIA-friendly:
- **Calculator** (`calc.exe`) — press digit/operator buttons by Name, read the result `Text` element. Deterministic, no network, ideal smoke test.
- **Notepad** — `set_value` into the `Edit` control, verify via `ValuePattern.Value`.
- Media: **Windows Media Player** or the Store **Media Player** for a play test, but expect the same nondeterminism caveat as Apple Music — assert tool-level success, log playback as best-effort.

### 6. Known-different behaviors to watch for

- **Element naming:** Windows apps often populate `AutomationId` (stable) where macOS only had a visible title. Prefer matching `Name`, fall back to `AutomationId`.
- **Chromium/Electron apps** (Slack, Discord, VS Code, Spotify desktop): on Windows these expose a partial UIA tree by default; some require the app to have accessibility enabled. Same class of limitation as the macOS `chromiumAppPatterns` special-casing already in `helper.rs`.
- **Focus/foreground:** UIA generally doesn't require foregrounding to read/invoke, like macOS AX. No CGEventPost-style CEF crash risk because UIA Invoke is not synthetic input injection.

### Quick start for the Windows machine

1. `launch_app` should already work — test `"open notepad"` / `"open calculator"` first.
2. Do NOT add launchers to `READ_ONLY_BASES` — `launch_app` (gated, URI-rejecting) is the Windows app-launch path. `Start-Process` lives inside that tool, not the shell allowlist.
3. Build `uia_interact.rs` against the `uiautomation` crate, mirroring the three `ax_interact` fns.
4. cfg-dispatch in `accessibility/mod.rs` so `ax_interact` the tool resolves to UIA on Windows.
5. Smoke-test with Calculator (deterministic), then a media app (best-effort).

### Cross-platform compatibility audit (current state)

Every Phase 1 change was written to **compile on all platforms** — nothing here breaks the Windows build. macOS-specific native code is `#[cfg(target_os = "macos")]`-gated and the non-macOS branches return a clean `"…macOS-only"` error at runtime rather than failing to build.

| Change | Cross-platform status | Notes for Windows |
|---|---|---|
| Auto-send dictation transcript (TS) | ✅ Fully portable | Pure frontend; no OS code. Works as-is. |
| `launch_app` | ✅ macOS / Linux / Windows branches | Windows branch now uses `Start-Process` with a **Store/UWP (`Get-StartApps` AUMID) fallback** and injection-safe env passing (§1). |
| `ax_interact` **tool** (`tools/impl/computer/ax_interact.rs`) | ✅ Functional on Windows | Delegates to `accessibility::ax_interact` fns, which now cfg-dispatch to the UIA backend on Windows. Description made OS-neutral. |
| `accessibility::ax_interact` helpers | ✅ cfg-dispatched | macOS → Swift helper; Windows → `uia_interact.rs`; other → clean runtime error. |
| `accessibility::uia_interact` (NEW) | ✅ Windows backend | UIA `list`/`press`/`set_value` via the `uiautomation` crate; same fn signatures as the macOS path. |
| Swift unified helper (`accessibility/helper.rs`) | ⚠️ macOS-only by design | Windows needs no helper process — UIA COM API is called directly from Rust. |
| App launching | ✅ Done | Launchers stay out of `READ_ONLY_BASES`; `launch_app` (gated) handles Windows `Start-Process`. |
| Notch indicator (separate PR #3166) | ⚠️ macOS NSPanel | A Windows equivalent would be a borderless always-on-top WebView2 window or a tray flyout — out of scope for this branch. |

**Before merging the Windows port, confirm the whole branch still builds and runs on macOS too** (`cargo check` on both `Cargo.toml` and `app/src-tauri/Cargo.toml`) so the cfg-dispatch doesn't regress the working macOS path.

### ⚠️ Mandatory: extensive E2E testing on Windows

The macOS path was hardened only through repeated real-app runs (each bug — CEF crash, select-vs-play, list truncation/hallucination — surfaced only by actually driving live apps, not by unit tests). **Do the same on Windows before considering it done.** Treat the following as the required E2E matrix:

1. **App launch** — `launch_app` for: a Win32 app (Notepad), a Store/UWP app (Media Player / Spotify from Store), and a URI (`spotify:`). Confirm each actually opens.
2. **Deterministic UI control** — Calculator: `list filter='5'` → `press '5'`, `press '+'`, `press '='`, then read the result element. Assert the computed value. This is the Windows analog of the AC/DC test and should be a **hard-asserted** automated test (Calculator is deterministic).
3. **Text entry** — Notepad: `set_value` into the Edit control, verify via `ValuePattern.Value`.
4. **Filtered list correctness** — confirm `list` with a `filter` returns a small, accurate set (the truncation→hallucination bug must not recur; verify the 60-element cap + filter behaves on a busy app like Settings or a browser).
5. **Real-world app** — drive a media app end-to-end (open → search → play). Assert tool-level success; treat playback state as **best-effort** (same nondeterminism caveat as Apple Music).
6. **Chromium/Electron app** — Slack/Discord/VS Code: confirm whether their UIA tree is exposed; document any app that needs accessibility explicitly enabled.
7. **Permissions/elevation** — verify behavior against a normal app vs an elevated one (UIPI boundary); document what fails and why.
8. **Agent-in-the-loop run** — the real test: ask the running agent (chat) to perform each action and confirm it picks `launch_app` / `ax_interact` and the action lands. Watch `[ax_interact]`/`[launch_app]` logs exactly as we did on macOS.
9. **Regression** — re-run the macOS E2E suite after the Windows changes land to prove cfg-dispatch didn't break the Mac path.

Add the deterministic ones (Calculator, Notepad, launch) as `#[cfg(target_os = "windows")]` `#[ignore]` integration tests mirroring `ax_interact_tests.rs`, runnable with `cargo test ... -- --include-ignored` on the Windows machine.

### Windows port — implementation status ✅

Shipped on the Windows machine (2026-06-02):

**Code**
- `Cargo.toml` — `uiautomation = "0.25"` under `[target.'cfg(windows)'.dependencies]`; `Win32_System_Com` feature added to `windows-sys` for COM init.
- `src/openhuman/accessibility/uia_interact.rs` (**new**) — UIA backend. `list` / `press` / `set_value` over the UIA COM tree via the `uiautomation` crate, mirroring the macOS `ax_interact` fn signatures. `press` activates via UIA patterns in order — `Invoke` → `SelectionItem.Select` → `LegacyIAccessible.DoDefaultAction` — never injecting synthetic input. `set_value` finds an editable field, preferring `Edit`, then `ComboBox`, then `Document` (so the Win11 RichEdit Notepad, whose editor is a `Document`, works). Exact-label match preferred over substring. Per-thread COM init via `CoInitializeEx(MTA)`.
- `src/openhuman/accessibility/ax_interact.rs` — the three public helpers now cfg-dispatch: macOS → Swift helper, Windows → `uia_interact`, else → clean runtime error. Module + tool docs made OS-neutral.
- `src/openhuman/accessibility/mod.rs` — declares `uia_interact` (cfg-gated to Windows).
- `src/openhuman/tools/impl/computer/ax_interact.rs` — description rewritten to be platform-neutral ("platform accessibility API (macOS AXUIElement / Windows UI Automation)").
- `src/openhuman/tools/impl/system/launch_app.rs` — Windows launcher hardened: app name passed via env var (no string interpolation → no injection), `Start-Process` first, then Store/UWP fallback by display name via `Get-StartApps` → AUMID (`shell:AppsFolder\<AppID>`); stderr surfaced on failure.
- `src/openhuman/security/policy_command.rs` — launchers (`open`/`xdg-open`/`start`) deliberately kept OUT of `READ_ONLY_BASES`; `launch_app` is the gated launch path.
- `src/openhuman/accessibility/uia_interact_tests.rs` (**new**) — `#[cfg(all(test, target_os = "windows"))]` integration tests, wired into `ax_interact.rs`.

**Test evidence (real apps on Windows 11)**
- `test_uia_calculator_five_plus_five` ✅ — drove the live Calculator entirely by element label: `list` → 41 interactive elements; pressed `Five`/`Plus`/`Five`/`Equals`; **hard-asserted** the readout `[Text] "Display is 10"` (5 + 5 = 10). Deterministic — the Windows analogue of the macOS AC/DC test.
- `test_uia_notepad_set_value` ✅ — `set_value` wrote into the live Win11 Notepad's `Document` "Text editor" (`Ok("Set 'Text editor' in 'Notepad' to the provided value.")`). The `Document` fallback is what makes the redesigned Notepad work.
- `test_uia_list_nonexistent_app` ✅ (non-ignored) — exercises COM init + window walk + error path deterministically.
- `launch_app` (×8) and `ax_interact` tool (×4) unit tests ✅.
- Full `cargo test --lib --no-run` compiles clean on Windows (warnings only, all pre-existing).

**Environment gotcha (this machine):** Norton real-time protection blocks `link.exe` from writing the freshly-linked ~150 MB test `.exe` (LNK1104, "Access denied" creating the file). Fix: exclude the repo's `target` dir under Norton's **Auto-Protect / SONAR / Download Intelligence** exclusion list (not the separate "Scans" list), and restore the file from Quarantine if already flagged.

**Follow-ups / not done here**
- **macOS regression check** — the cfg-dispatch is additive (the `#[cfg(target_os="macos")]` arms are untouched; only the non-macOS catch-all message changed), but per the branch note, re-run `cargo check` + the macOS E2E suite on a Mac before merge to prove the Mac path didn't regress (can't be done from the Windows machine).
- **Agent-in-the-loop run** (E2E item 8) — the full Tauri desktop app was built and run on Windows (`pnpm dev:app:win`) and the in-process core booted fine (verbose `[launch_app]`/`[ax_interact]`/`[uia_interact]` logging wired via `RUST_LOG`). The first chat attempt couldn't complete because the configured **local AI model was still downloading** (`kind="empty_provider_response"` — the agent returned an empty response, so it never reached a tool call). **Still pending:** a working model (finish the local download or select a configured cloud model), then ask the agent "open Calculator" / "press 5 in Calculator" and confirm it picks `launch_app`/`ax_interact` and the action lands.
- Chromium/Electron UIA coverage, elevation/UIPI behavior, and a busy-app filtered-list check (E2E items 4/6/7) remain to be spot-checked manually.

---

## Phase 2 — Always-On Listening ✅ Implemented

> Continuous microphone listening without requiring a hotkey press.

**Shipped:**
- `src/openhuman/voice/always_on.rs` — pure `VadSegmenter` (onset / silence-hangover / min-speech / max-utterance, 7 unit tests) **plus** the continuous capture loop: a dedicated cpal thread streams 16 kHz mono frames → segmenter → each utterance is encoded (`encode_wav_16k`) → `voice_transcribe_bytes` → `publish_transcription` (so it reaches the agent's auto-send and the notch, exactly like hotkey dictation). Started at boot in `credentials::ops`.
- `src/openhuman/config/schema/voice_server.rs` — `always_on_enabled` flag + VAD tuning (`vad_onset_threshold`, `vad_hangover_ms`, `vad_min_speech_ms`, `vad_max_utterance_secs`), opt-in/off by default.
- **Settings toggle** — "Always-on listening" in the Voice debug panel, wired through `get/update_voice_server_settings` (RPC patch → apply → snapshot); i18n in en + all 13 locales.
- **Privacy hook** — `spawn_lock_watcher` pauses capture + resets the segmenter while the screen is locked (macOS via `CGSessionCopyCurrentDictionary`, null/type-safe FFI; other platforms never pause yet).
- Reused `audio_capture` helpers (`to_mono`/`resample`/`chunk_rms` made `pub(crate)` + new `encode_wav_16k`).

**Acceptance criteria:**
- [x] User can speak without pressing any hotkey
- [x] VAD detects end of utterance and sends to agent
- [x] Toggle in Settings → Voice

**Wake word "Hey Tiny" (live-fix, 2026-06-03):** always-on now only delivers an utterance to the agent when its transcript contains the wake word (`config.voice_server.wake_word`, default "Hey Tiny"); the phrase is stripped and the remainder is sent. Tolerant match (case/punctuation/leading-filler), empty wake word = deliver everything. This is a **text-based** wake word (transcribe-then-gate) — a first cut of Phase 3's trigger phrase; it fixes the "sends every utterance" spam but still runs STT on all speech (an on-device audio wake-word model for efficiency is the Phase 3 follow-up).

**Live-fixes found by running it:**
- **Toggle did nothing** — `always_on_enabled` wasn't in the `update_voice_server_settings` RPC *param schema*, so validation rejected it before the handler. Added it; the config RPC now also calls `always_on::start_if_enabled` so the toggle starts/idles capture **live** (runtime `ENABLED` gate, no restart).
- **`transcription failed: local ai is disabled`** — always-on used `voice_transcribe_bytes` (local whisper only). Now routes through `effective_stt_provider` + `create_stt_provider` (same factory dispatch as `voice.stt_dispatch`), honoring cloud STT.
- Toggle surfaced in the reachable **VoicePanel** (Settings → Advanced → AI → Voice), not the hidden debug panel.

**Pending live validation (mic-dependent, can't be CI-tested):** say "Hey Tiny, <command>" and confirm the command reaches the agent; tune `vad_onset_threshold`/`vad_hangover_ms` to the user's mic + room. Windows/Linux screen-lock pause is a follow-up (no signal wired).

---

## Phase 3 — Wake-Word + Fast Routing 🔨 Fast routing shipped; audio wake-word model pending

> Fast command routing is **merged** ([#3346](https://github.com/tinyhumansai/openhuman/pull/3346)). The single remaining Phase 3 item is the on-device **audio** wake-word model — **not started**; the text-based "Hey Tiny" transcribe-then-gate match from Phase 2 is the interim.

**Fast routing — DONE & WIRED:** `src/openhuman/voice/command_router.rs` (new) — pure `route(transcript) -> VoiceIntent` classifier (Play/Pause/Resume/Next/Previous/OpenApp/SetVolume/VolumeUp·Down/Mute/Unmute, else `Unknown`). High-confidence intents execute directly (launch_app / Music fast-path / osascript volume) without a full chat-LLM turn — the ≤500 ms path; `Unknown` defers to the agent so routing only ever shortcuts. Filler-tolerant ("please open up slack"). 5 unit tests.

**Router wired into delivery:** `always_on::deliver_command` now routes the extracted command through `command_router::route` first. Recognized intents run locally via `execute_intent` (Play → `automate` Music fast-path; OpenApp → `launch_platform`; Pause/Resume/Next/Previous/volume/mute → `osa` osascript); `VoiceIntent::Unknown` (and any local-exec failure) falls back to the full agent turn. Verified live: clean transport/open/volume commands execute on the fast path; complex/garbled `play` queries (e.g. mangled STT like "bts s latest song wind blowing") fall through to the agent as designed. Committed on `feat/voice-phase3-fast-routing`.

**Remaining Phase 3:** on-device audio wake-word model (Porcupine / ONNX) in `inference/voice/wake_word.rs` to gate STT *before* transcription — the text-based "Hey Tiny" wake match from Phase 2 (fuzzy anchor on the "tiny" token, Levenshtein ≤1) is the interim. This is the only open Phase 3 item; not yet started.

---

## Phase 3 — Wake-Word + Fast Routing (original plan)

> Activate only on a trigger phrase; route simple commands locally without a full LLM turn.

**Planned files:**
- `src/openhuman/inference/voice/wake_word.rs` (new) — lightweight always-on model (Porcupine or custom ONNX) — ⏳ **not started** (interim: text-based "Hey Tiny" match in Phase 2)
- `src/openhuman/voice/command_router.rs` (new) — intent→tool mapping for high-confidence commands, LLM fallback for ambiguous input — ✅ **shipped & wired** (see Phase 3 status above)

**Acceptance criteria:**
- [ ] Wake-word detection runs fully on-device (still pending — audio wake-word model not yet built)
- [x] Latency from end-of-utterance to action start ≤ 500ms for local-routed commands (command router executes recognized intents on the local fast path)

---

## Phase 4 — Polish ⏳ Not Started

> Voice confirmation loop, UI indicator, computer control onboarding.

**Planned:**
- TTS confirmation before executing sensitive actions ("Opening Music — confirm?")
- Always-on status indicator (notch pill from PR #3166 will handle this automatically)
- Computer control (`mouse`/`keyboard` tools) toggle in Settings onboarding

---

## Fine-tuning backlog ⏳ (deferred until all phases complete)

From live agent-in-the-loop testing on 2026-06-03 (grounded in `~/.openhuman/logs/openhuman.2026-06-03.log`, `session_raw/*.jsonl`, and the dev run: **keyboard=69 / mouse=0 / screenshot=10** tool calls; **26 wake matches vs 93 misses**; emit=true utterances ranged 0.7s–28s). The feature works but needs tuning. **Do not implement until Phases 3–4 land.**

### F1 — Listening window too short for long commands
- **Observed:** `vad_hangover_ms = 800` closes an utterance on any pause > 0.8s, so multi-clause commands ("Hey Tiny, open Slack and message the team channel saying …") split across utterances — the tail lacks the wake word and is dropped. Compounded by the notch "Listening" pill TTL (2500ms) expiring mid-speech, so it *looks* like it stopped listening.
- **Resolve:** (a) raise `vad_hangover_ms` to ~1500ms; (b) **two-stage capture** — once the wake word is detected, open a dedicated longer command window (until a longer silence / N-second cap) instead of relying on a single VAD utterance; (c) keep the "Listening" pill alive for the whole utterance (extend/re-emit on each voiced frame, clear on `SpeechEnd`) so the notch reflects real mic state.

### F2 — Agent uses keyboard only, never the mouse
- **Observed:** keyboard=69, mouse=0. Two causes: the orchestrator prompt is deliberately *keyboard-first*, **and** the downscaled screenshot's coordinates don't map to screen pixels — the capture is shrunk to ≤1568px while `mouse` expects absolute screen pixels (and Retina is 2× points), so any coordinate read from the image clicks the wrong spot. Vision-driven clicking is therefore currently unsafe and the agent (correctly) avoids it.
- **Resolve:** (a) make `screenshot` emit a coordinate transform (shown WxH + real screen WxH + backing scale) **or** have `mouse` accept image-relative coordinates and convert internally; (b) once coordinates are trustworthy, soften the prompt so the agent uses screenshot→mouse to click specific elements, not just keyboard.

### F3 — No periodic screenshot/verify + foreground re-check
- **Observed:** the agent screenshots ad-hoc (0 in the last session); `automate` only foregrounds at the start.
- **Resolve:** in the `automate` loop **and** the orchestrator prompt — screenshot + verify at **start, after every ~3 actions, and at the end**; before each action confirm the frontmost app is the target and re-`launch_app` (foreground) it if not, then proceed. Fold the actual-vs-expected check into the loop's `verify` step.

---

## Permissions matrix — what the desktop agent needs to run

> Grounded in `src/openhuman/accessibility/permissions.rs` (detection),
> `src/openhuman/accessibility/types.rs` (`PermissionKind`),
> `app/src-tauri/Info.plist` (usage strings), and
> `app/src-tauri/entitlements.sidecar.plist` (Hardened-Runtime entitlements).
> Use this as the pre-test checklist when building for macOS (Apple Silicon +
> Intel) and Windows.

### The four OS permissions the app tracks

`detect_permissions()` returns a `PermissionStatus` over these four
`PermissionKind`s:

| Permission | Detected via (macOS) | Agent capability that needs it |
|---|---|---|
| **Microphone** | CPAL input-device probe (`detect_microphone_permission`) | All voice capture — dictation hotkey **and** always-on listening (`voice/always_on.rs` mic stream). **Required for "voice on" to capture anything.** |
| **Accessibility** | `AXIsProcessTrusted()` | The whole app-control surface: `ax_interact` (AX tree read/press), `automate`, autocomplete focus query, **and synthetic keyboard/mouse injection** (enigo / CGEvent). |
| **Screen Recording** | `CGPreflightScreenCaptureAccess()` | `screenshot` → `vision_click` (vision fallback for Electron/partial-AX apps) and screen-intelligence capture. |
| **Input Monitoring** | `IOHIDCheckAccess(LISTEN_EVENT)` | Global hotkey **listening** (dictation hotkey / rdev / Globe-key listener — `accessibility/keys.rs`, `globe.rs`). Not needed for always-on (no hotkey). |

### Supporting macOS entitlements + usage strings (Hardened Runtime)

These are baked into the **signed build**, not runtime toggles. If missing, the
OS blocks the call *before* any consent dialog renders — so they must be correct
in the signed `.app`/DMG, and **cannot be validated with `tauri dev` alone**.

- **`Info.plist` usage strings:** `NSMicrophoneUsageDescription`,
  `NSCameraUsageDescription`, `NSAppleEventsUsageDescription` (+ Bluetooth /
  Location / Contacts / Calendar / folder strings — those are mostly for the
  **embedded provider webviews**, not the agent itself).
- **`entitlements.sidecar.plist`:** `device.audio-input`, `device.camera`,
  `automation.apple-events` (drives `osascript` / System Events — Music
  transport, volume, foreground detection), `network.client` / `network.server`.

### Capability → permission map (agent surfaces)

| Capability | Microphone | Accessibility | Screen Recording | Apple Events / Automation | Input Monitoring |
|---|:---:|:---:|:---:|:---:|:---:|
| Always-on capture (mic → VAD → STT) | ✅ | — | — | — | — |
| Dictation hotkey | ✅ | — | — | — | ✅ (listen) |
| `launch_app` / `OpenApp` intent | — | — | — | — | — |
| `Pause`/`Next`/volume intents (osascript) | — | — | — | ✅ | — |
| `Play` (Music fast-path) | — | ✅ | — | ✅ (verify `player state`) | — |
| `ax_interact` (list/press/set_value) | — | ✅ | — | — | — |
| `automate` general loop | — | ✅ | ✅ (only for `vision_click`) | ✅ (where it uses osascript) | — |
| `vision_click` (Electron fallback) | — | ✅ | ✅ | — | — |
| `keyboard` / `mouse` (synthetic input) | — | ✅ | — | — | — |

**Minimal "voice on" path:** Microphone to capture; then per intent —
`OpenApp` needs nothing extra, transport/volume need Automation, `Play` needs
Accessibility + Automation.

### Per-OS differences (the three test machines)

| | macOS (M-chip & Intel — identical) | Windows | Linux |
|---|---|---|---|
| **Microphone** | Real TCC prompt; **does** prompt | Real privacy gate — Settings → Privacy → Microphone | Ungated on standard desktops; Flatpak needs an XDG portal |
| **Accessibility** | Manual grant in System Settings → Privacy & Security; **does not reliably auto-prompt**; usually needs an app **restart** after granting | No analog — UIA needs **no permission** for same-integrity apps; **cannot drive elevated apps** (UIPI) | App-interaction effectively **unsupported** (backend is macOS Swift helper / Windows UIA only; Linux returns a clean runtime error) |
| **Screen Recording** | Manual grant; no reliable auto-prompt | `screenshot`/`vision_click` is **currently unimplemented on Windows** — the vision fallback can't run there yet | Unsupported |
| **Input Monitoring** | Manual grant for hotkey listening | Not required | Not required |
| **Entitlements** | Must be present in the signed build | n/a | n/a |

### Gotchas to flag before testing

1. **Detection ≠ entitlement.** On macOS the runtime checks report
   `Denied`/`Unknown` if the *entitlement* is missing from the signed build,
   independent of what the user clicked — so test on a **properly signed build**,
   not `tauri dev`.
2. **Microphone detection is a proxy.** `detect_microphone_permission` infers
   from CPAL device enumeration, so "no device connected" and "permission denied"
   both surface as `Unknown` on macOS/Windows (`permissions.rs`). The always-on
   capture logging (`[voice::always_on] microphone permission: …` + device name +
   first-chunk confirmation) disambiguates this live.
3. **Restart after granting** Accessibility / Screen Recording / Input
   Monitoring on macOS — TCC changes are not always picked up by a running
   process.

---

## Summary

| Phase | Item | Status |
|---|---|---|
| 1 | Auto-send after transcription | ✅ Done |
| 1 | Shell allowlist for `open`/`xdg-open` | ✅ Done |
| 1 | Shell tool description clarification | ✅ Done |
| 1 | Dedicated `launch_app` tool | ✅ Done |
| 1 | Orchestrator tool scope | ✅ Done |
| 1 | SOUL.md capability hint | ✅ Done |
| 1 | Diagnostic logging | ✅ Done |
| 1 | Computer control (mouse/keyboard) | ❌ Reverted (CEF crash) |
| 1 | AXUIElement app UI interaction (`ax_interact`) | ✅ Done |
| 1 | Multi-step UI workflow guidance | ✅ Done |
| 1 | Apple Music two-step play (navigate→play) | ✅ Done (playback best-effort) |
| 1 | Full computer control (mouse/keyboard/screenshot) — CEF crash (Change 1.15) | ✅ Fixed (main-thread synthetic-input dispatch; screenshot downscale) |
| 1 | Windows port (`launch_app` + UIA `ax_interact`) | ✅ Done (Calculator + Notepad hard-asserted on Win11) |
| 1 | `automate(app, goal)` Rust-driven loop (Change 1.14) | ✅ M1–M5 done (proven live on macOS) |
| 1.5 | M1: automate loop skeleton + tool | ✅ Done |
| 1.5 | M2: poll-until-stable settle | ✅ Done |
| 1.5 | M3: Music fast-path | ✅ Done (proven live on macOS) |
| 1.5 | Robustness: quoted-query parse + no-progress guard | ✅ Done (from live agent failures) |
| 1.5 | M4: progress streaming to notch | ✅ Done — notch cherry-picked in; automate streams live steps |
| 1.5 | M5: richer element model (`enabled`) | ✅ Plumbed; AXEnabled found unreliable → informational only |
| 1.5 | Native fast-paths beyond Music (Spotify/Slack) | ➖ Descoped (general loop covers them; Music shipped in M3) |
| 1.5 | Vision fallback for Electron apps (Change 1.16) | ✅ Done (`vision_click`: screenshot → vision-locate → guarded click; frontmost guard; F2 coord-transform folded in) |
| 1.5 | Browser fast-path (Change 1.17a) | ✅ Done (`browser.rs`: deterministic `open -a "<browser>" url` nav; play→fall-through to vision_click) |
| 1.5 | Cross-platform browser shortcut table (Change 1.17b) | ✅ Done (`browser_shortcuts.rs`: Chrome/Firefox/Safari/Edge × macOS/Win/Linux) |
| 1.5 | App-shortcut fast-path: Spotify/Apple Music/Slack (Change 1.17c) | ✅ Done (`app_shortcuts.rs`: transport + nav via each app's shortcuts) |
| 1.5 | Keyboard-shortcut plumbing: `hotkey` verb + backend methods (Change 1.17d) | ✅ Done (`key`/`type_text`/`open_url_in_app`/`now_playing`; shared keyboard helpers) |
| 1.5 | Artist-aware Music verification (Change 1.17e) | ✅ Done (`now_playing` osascript; honest wrong-artist report, no false success) |
| 1.5 | Actionable `automate` responses (Change 1.17f) | ✅ Done (next-step guidance, candidate/label lists, verified-only success, song-row preference) |
| 2 | Always-on microphone loop | ✅ Done (cpal → VAD → STT → agent) |
| 2 | `always_on_enabled` config flag + Settings toggle | ✅ Done (RPC + UI + i18n) |
| 2 | Privacy hook (screen lock pause) | ✅ Done (macOS; other OSes follow-up) |
| 2 | Text-based "Hey Tiny" wake word | ✅ Done (interim; gates delivery, strips phrase) |
| 3 | Local command router (intent classifier) | ✅ Done & wired (recognized intents run on the ≤500ms local path; Unknown defers to agent) |
| 3 | On-device audio wake-word model | ⏳ Not started (text-based match is the interim) |
| 4 | Voice confirmation loop | ⏳ Not started |
| 4 | Computer-control onboarding toggle | ⏳ Not started |
| 4 | Always-on UI indicator | ✅ Done (notch PR #3166) |
