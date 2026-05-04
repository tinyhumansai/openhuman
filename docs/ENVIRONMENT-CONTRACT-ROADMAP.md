# Environment Contract Roadmap

Post-v1 direction. Framing borrowed from Jeffrey Li's "Agent Harness Is Not Enough"
(holaOS thesis): long-horizon agent systems need an *environment contract* around
the execution harness, not just a better harness.

This doc is the note-version of where we go **after** v1 ships.
Not a replacement for `TODO.md` — that stays tactical.

---

## Where we already sit on the contract

| Contract layer | Today in openhuman |
| --- | --- |
| Durable authored state | `skills/` submodule, `ai/*.md` (SOUL, IDENTITY, AGENTS, USER, BOOTSTRAP, MEMORY, TOOLS), controller registry (`src/core/all.rs`) |
| Durable adaptive state | TinyHumans memory (`skill-{skill}` namespaces, with `integration_id` carried in record metadata), curated_memory snapshots, retrieval evals |
| Runtime continuity | `OPENHUMAN_WORKSPACE` override, r2d2 SQLite pools, life_capture ingest, event bus |
| Projected execution state | Controller schemas, JSON-RPC dispatch, capability routing per run |
| Portability | Workspace-as-unit via `OPENHUMAN_WORKSPACE` |

The harness (Rust agentic loop in `src-tauri/src/commands/chat.rs`) is swappable.
Most of the weight is already in environment, not in the loop.

---

## Gaps to close (the "review boundary")

Order matters: each unlocks signal for the next.

### 1. Run trace persistence  *(unlocks everything else)*
Today: eval traces exist as fixtures; run-level traces are ephemeral.
Need:
- Persist per-turn record: hot context composition (what was pulled from memory /
  OpenClaw / Notion), tool calls fired + results, model routing, outcome.
- Land in local SQLite under workspace root (`OPENHUMAN_WORKSPACE/traces/`).
- Surface in UI (traces panel) — operator can inspect a run later.
- Keep it cheap: append-only, no sync by default.

Why first: no review loop works without durable evidence of what happened.

### 2. Operator feedback primitives
Today: feedback is implicit (user edits, re-runs, disconnects).
Need:
- Explicit signals on: memory candidates (keep/drop), tool results (good/bad),
  full turns (thumbs). Minimal UI — thumb + optional reason string.
- Feedback attaches to trace ID so signal is joinable with context.
- Stored alongside traces; no backend dependency.

Why second: traces without judgment are noise. This is the reward-like signal
Jeffrey calls out.

### 3. Curated_memory → candidate skill pipeline
Today: curated_memory promotes facts into prompts. No path from "agent did X
reliably" to "X is a skill."
Need:
- Detect repeated tool-call patterns with positive feedback (e.g. same sequence,
  same shape of args, good outcomes).
- Generate candidate skill scaffold (`SKILL.md` with frontmatter per the current loader contract; legacy `skill.json` remains as a fallback only).
- Review queue in UI — user approves, rejects, or edits before it lands in
  `skills/`.
- Promoted skill is just a regular skill from that point on.

Why third: needs (1) for pattern data and (2) for "reliably" judgment.

### 4. Capability projection per role
Today: controller permissions and visibility are static.
Need:
- Roles as first-class: "trading assistant," "inbox triage," etc., each with its
  own allowed action surface.
- Capability grants tied to review — role earns a skill/tool only after the
  candidate pipeline promotes it.
- Per-run projection: harness only sees the surface the role owns.

Why last: hardest and needs (1)-(3) to have signal worth projecting from.

---

## Non-goals

- **Not** replacing the Rust harness. The loop is fine; the point is the
  contract around it.
- **Not** building a generic agent OS. openhuman is a product (AI assistant for
  communities); the contract serves that.
- **Not** shipping this before v1. Premature without real usage data — the whole
  point is review over runs that actually happened.

---

## Harness-swap test (our rubric)

If we replaced `chat_send_inner` with Claude Agent SDK or OpenAI Agents SDK
tomorrow, these must survive unchanged:

- [x] Skills manifests + handlers
- [x] Memory namespaces + curated snapshots
- [x] Controller registry + JSON-RPC schemas
- [x] Event bus + life_capture data
- [x] Workspace portability (`OPENHUMAN_WORKSPACE`)
- [ ] Run traces (missing)
- [ ] Operator feedback records (missing)
- [ ] Promoted skill provenance (missing)
- [ ] Role → capability map (missing)

v1 closes the first five. This roadmap closes the last four.

---

## Open questions

- Where do traces live long-term? Local-only, or opt-in sync for eval?
- Does role modeling need UI, or is it config-only to start?
- Candidate skills: LLM-generated scaffold vs. pure pattern extraction?
- Do we expose traces to skills themselves (self-improvement loop) or keep them
  operator-only?

---

_Seeded 2026-04-22 after conversation on Jeffrey Li's environment-contract piece._
_Sequencing and scope will shift once v1 is in real users' hands._
