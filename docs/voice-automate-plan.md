# Phase 1.5 Implementation Plan — `automate(app, goal)`

**Parent tracker:** [`voice-system-actions.md`](voice-system-actions.md) (Change 1.14 / Phase 1.5)
**Decided approach:** Rust inner loop + fast model (chat LLM out of the click loop)
**First proof target:** Music — "play `<song>`" end-to-end
**Status:** Plan — awaiting approval before code

---

## 1. Goal

Turn a single high-level intent ("play Numb by Linkin Park") into a multi-step UI
automation that completes in **one tool call from the orchestrator**, runs fast,
and self-corrects — instead of N separate chat-LLM turns over the raw
`ax_interact` primitives (today's flow; see tracker §1.10–1.13 for why that's
slow and fragile).

## 2. Architecture

```text
 orchestrator (chat LLM)
        │  one call: automate{ app, goal }
        ▼
 AutomateTool (tools/impl/computer/automate.rs)
        │  delegates to
        ▼
 accessibility::automate::run(app, goal)         ← the inner loop (Rust)
        │
        ├─ fast-path dispatch ── app_fastpaths/{music,spotify,slack}.rs
        │      (deterministic; skip the loop entirely when available)
        │
        └─ general loop ──► perceive → decide → act → settle → verify ──┐
               ▲                                                         │
               └────────────── repeat until done / fail / budget ───────┘
                 perceive: ax_list_elements_filtered (existing)
                 decide:   create_chat_provider("automation", cfg) → JSON action
                 act:      ax_press_element / ax_set_field_value / launch_app (existing)
                 settle:   helper "ax_wait_settled" (new) — AXObserver, not sleep
                 verify:   re-read state; confirm the action took effect
```

The **chat model is invoked once** (to pick `automate` and its `goal`). The
**fast model** runs the inner loop with a tiny context (goal + current filtered
snapshot + last result), so each step is ~0.5–1s and cheap.

## 3. Inner-loop algorithm

State carried across iterations: `goal`, `app`, `history: Vec<Step>`, `budget`.

Each iteration:
1. **Perceive** — `ax_list_elements_filtered(app, last_filter_or_"")`, capped/filtered
   exactly as the `ax_interact` tool does today (≤60 elements, never a raw dump).
2. **Decide** — call the fast model with a strict system prompt + the JSON action
   schema (below). Parse one action.
3. **Act** — execute via existing helpers. `launch` → `launch_app`; `press` →
   `ax_press_element`; `set_value` → `ax_set_field_value`; `list` → just re-perceive
   with a new filter.
4. **Settle** — `ax_wait_settled(app, timeout)` (new helper): block until the AX
   tree stops changing (debounced AXObserver notifications) or timeout. Removes the
   timing-race class deterministically.
5. **Verify** — re-read; confirm the expected post-condition (e.g. a new control
   appeared, focus changed, a value was set). Record success/failure in `history`.
6. **Loop** until the model emits `done`/`fail`, or the step budget (e.g. 12) is hit.

### Action schema (fast model output — strict JSON)
```jsonc
{
  "thought": "short reasoning",
  "action": "launch | list | press | set_value | done | fail",
  "app": "Music",            // optional override; defaults to the task app
  "filter": "Highway",       // for list
  "label": "Play",           // for press / set_value
  "value": "Highway to Hell", // for set_value
  "summary": "what happened / why done"  // for done|fail
}
```
Invalid JSON or unknown action → one repair retry, then `fail` with the raw text
logged (never act on a guess — this is the §1.13 hallucination lesson).

## 4. New files & changes (grounded in current layout)

**New**
- `src/openhuman/accessibility/automate.rs` — `run(app, goal, opts) -> Result<AutomateOutcome, String>`; the loop, action schema (serde), fast-model call, step budget, structured `history`.
- `src/openhuman/accessibility/app_fastpaths/mod.rs` + `music.rs` (Spotify/Slack land later) — `try_fastpath(app, goal) -> Option<Result<…>>`.
- `src/openhuman/tools/impl/computer/automate.rs` — `AutomateTool { allow_mutations }`; reuses the `ax_interact` gating posture (mutations opt-in, `SENSITIVE_APPS` denylist, `permission_level_with_args` = Dangerous, `external_effect_with_args` = true).
- `src/openhuman/accessibility/automate_tests.rs` — unit tests for the loop (mock perceive/act/decide), schema parse/repair, budget, fast-path dispatch.

**Changed**
- `accessibility/helper.rs` (macOS Swift) — add `ax_wait_settled` (AXObserver on `kAXValueChanged`/`kAXFocusedUIElementChanged`/`kAXCreated`, debounce ~150ms, bounded ~3s) and return richer element fields (enabled / on-screen / supported actions) from `ax_list`.
- `accessibility/ax_interact.rs` — surface a `ax_wait_settled` Rust wrapper; extend `AXElement` with the new optional fields (back-compat: `#[serde(default)]`).
- `accessibility/mod.rs` — declare `automate`, `app_fastpaths`.
- `inference/provider/factory.rs` — add an `"automation"` role (falls back to the fast/summarization tier) so the loop's model is independently configurable.
- `tools/ops.rs` (`all_tools_with_runtime`), `tools/user_filter.rs` (new `"automate"` family), `agent_registry/agents/orchestrator/agent.toml` (`named` list), `app/src/utils/toolDefinitions.ts` (Settings → Agent Access toggle).
- Tracker: flip Change 1.14 / Phase 1.5 rows from ⏳ Planned → in progress as milestones land.

## 5. Fast-model call

`create_chat_provider("automation", &cfg)` → `(provider, model)`; build a
`ChatRequest { messages, tools: None, stream: None }` with a system prompt that
pins the JSON schema and a user message carrying `{goal, snapshot, history_tail}`.
No tools array — we want a single JSON object back, parsed by us, executed by us.
Temperature low. Token budget small (snapshot is already ≤60 elements).

## 6. Music proof (first target)

`app_fastpaths/music.rs` encodes the §1.11 proven sequence behind one entry:
1. `launch_app("Music")`
2. open `music://music.apple.com/search?term=<query>` (URL scheme)
3. `ax_wait_settled`
4. `ax_list_elements_filtered("Music", <query>)` → find the song row
5. `ax_press_element` the row (navigate into detail)
6. `ax_wait_settled` → `ax_list` the detail page → `ax_press_element("Play")`
7. verify `osascript … get player state == playing` (best-effort, logged)

If the fast-path can't find the row (timing/locale), fall through to the **general
loop**, which is what proves the architecture is app-agnostic.

## 7. Progress streaming

Emit a `DomainEvent` per step (`AutomateProgress { app, step, action, ok }`) on the
event bus; a subscriber bridges to the existing notch/voice status surface
(PR #3166) so the user sees "Opening Music → searching → playing" live. Reuses the
`ApprovalSurfaceSubscriber` bridging pattern.

## 8. Testing

- **Unit** (`automate_tests.rs`, CI-safe): action JSON parse + repair; budget exhaustion → `fail`; fast-path dispatch chosen over loop; verify-failure triggers retry/alternate. Perceive/act/decide are trait-injected so tests need no mic/AX/LLM.
- **Integration** (`#[ignore]`, run on a real Mac): the Music flow end-to-end (mirrors `ax_interact_tests::test_full_flow_search_and_play_acdc`); tool-level success hard-asserted, playback best-effort.
- **Agent-in-the-loop**: ask the running app "play `<song>`", confirm it picks `automate` and the song plays; watch `[automate]` logs.

## 9. Milestones (sequenced)

1. **M1** — `automate.rs` loop skeleton + action schema + fast-model call + `AutomateTool` (gated, registered). Loop runs against existing (non-settled) `ax_interact` helpers. Unit tests. *Compiles + agent can call it.*
2. **M2** — `ax_wait_settled` (helper + wrapper) + verify step wired into the loop. Kills the timing-race class.
3. **M3** — Music fast-path; prove the flow end-to-end on a Mac.
4. **M4** — progress streaming to the notch surface.
5. **M5** — richer element model (enabled/onscreen/actions) for better matching.
6. *(later)* Spotify + Slack fast-paths; vision fallback for Electron; Windows UIA settle parity.

## 10. Risks / open questions

- **Fast model availability** — if no fast tier is configured, fall back to the
  chat model for the loop (still one tool call; just slower). The `"automation"`
  role makes this a config decision, not a hard dependency.
- **AXObserver from the Swift helper** — needs a short run-loop pump; if flaky,
  fall back to a polling settle (count-stable-for-150ms) behind the same wrapper.
- **macOS-only first** — Windows UIA settle/verify parity is M6, gated like the
  existing cfg-dispatch; non-mac/non-win returns the existing clean runtime error.
- **Safety** — `automate` is a mutating tool: same opt-in + `SENSITIVE_APPS`
  denylist + ApprovalGate routing as `ax_interact`; the inner loop may not target a
  denylisted app even if the model asks.
