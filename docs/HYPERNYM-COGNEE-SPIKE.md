# Hypernym + Cognee Spike Plan

Post-v1. Gated on the Environment Contract roadmap having real run-trace data
(roadmap gap #1) — no point compressing or graphing zero traces.

These two tools pair: **Hypernym compresses traces into dense context; Cognee
graphs across them.** Spike them together on one subsystem, not the whole
memory layer.

---

## Target subsystem

**Environment Contract roadmap gap #3: candidate skill pipeline.**

Why this one:
- It's the one where TinyHumans Neocortex's flat namespaced memory is
  *actually insufficient* — detecting "agent did X reliably" needs
  cross-trace pattern matching, which is a graph query.
- It has a clear success metric (candidate skills proposed / accepted) vs.
  generic "context quality."
- It doesn't touch the hot inference path in `src/openhuman/agent/` — spike
  can run offline over persisted traces without regressing v1 UX.

Explicitly **not** the first target:
- Hot-path context retrieval in the agent loop (too much blast radius).
- Replacing `src/openhuman/memory/` backend (don't rip out Neocortex).

---

## Architecture: two sidecars, behind a trait

```text
traces/ (SQLite, from roadmap gap #1)
    │
    ▼
┌─────────────────────────────────────────────────┐
│ candidate_skill_pipeline (new, src/openhuman/)  │
│                                                  │
│   1. pull N recent traces with positive feedback │
│   2. → Hypernym: compress each trace summary     │  ◄── sidecar 1 (MCP)
│   3. → Cognee:   cognify into DataPoints +       │  ◄── sidecar 2 (HTTP/MCP)
│        edges (Trace → ToolCall → Entity)         │
│   4. graph query: repeated (tool_seq, arg_shape) │
│        with avg_feedback > threshold             │
│   5. emit candidate skill scaffold → review UI   │
└─────────────────────────────────────────────────┘
```

Both sidecars run locally. Self-hosted. No new cloud dependencies.

Wrap behind a Rust trait so either can be swapped without touching the
pipeline:

```rust
// src/openhuman/environment/compactor.rs
trait TraceCompactor { fn compress(&self, trace: &Trace) -> CompactTrace; }

// src/openhuman/environment/graph.rs
trait TraceGraph {
    fn ingest(&self, t: &CompactTrace) -> Result<()>;
    fn find_repeated_patterns(&self, min_support: u32, min_feedback: f32)
        -> Vec<Pattern>;
}
```

---

## Phased execution

**Phase 0 — gates (don't start until these are true):**
- [ ] Roadmap gap #1 landed: `OPENHUMAN_WORKSPACE/traces/` populated in real
      usage for ≥ 2 weeks.
- [ ] Roadmap gap #2 landed: feedback primitives attached to trace IDs.
- [ ] v1 shipped.

**Phase 1 — Hypernym only (1 week):**
- Stand up `hypernym-mcp-server` locally, register in `app/src/lib/mcp/`.
- Write an offline CLI: `openhuman trace-compact --since 7d` → compressed JSONL.
- Measure: tokens-in vs tokens-out ratio on 100 real traces. Spot-check that
  the compressed form still answers "what tools were called, what did the
  user want, did it work" in a blind eval (have Claude judge 20 samples).
- **Kill criterion:** if compression < 3x or blind eval loses > 15% fidelity,
  stop. Don't proceed to Cognee.

**Phase 2 — Cognee on top (1-2 weeks):**
- Run `cognee-mcp` as a second sidecar (uvx or docker).
- Define DataPoints: `Trace`, `ToolCall`, `Entity`, `Skill`, `FeedbackSignal`.
- Ingest Phase 1's compressed traces via `cognee.add` → `cognee.cognify`.
- Implement `find_repeated_patterns` as a Cognee graph query.
- Build minimal review UI: list of candidate patterns + "promote to skill"
  button that scaffolds a `<workspace>/.openhuman/skills/<id>/SKILL.md` draft
  (with frontmatter per the current SKILL.md-first loader contract; legacy
  `skill.json` remains as a fallback only).
- **Success criterion:** at least one candidate skill scaffold from real
  traces that the operator would accept (even with edits).

**Phase 3 — decide (after 1 month of Phase 2 running):**
- If operator promotes ≥ 2 candidate skills from the pipeline: promote the
  trait impls to first-class, document in `docs/ARCHITECTURE.md`, consider
  expanding to gap #4 (role capability projection).
- If 0-1 promotions: keep Hypernym (it's useful independently for trace
  storage cost), drop Cognee, revisit when a Rust client exists.

---

## What we're measuring

Per phase, one number each:

| Phase | Metric | Threshold to continue |
| --- | --- | --- |
| 1 | Compression ratio + blind-eval fidelity loss | ≥ 3x, ≤ 15% loss |
| 2 | Operator-accepted candidate skills / month | ≥ 2 |
| 3 | Accepted candidates still active 30d later | ≥ 50% |

Each is cheap to collect and refutes the spike cleanly. No "feels better."

---

## Open questions for you

1. Do we want Hypernym compression on the **read** path (context packer, hot)
   or only on the **write** path (trace storage, cold)? Plan above is cold-only
   — much safer. Hot path is a separate spike.
2. Do candidate skills get generated by pattern extraction (deterministic) or
   by an LLM reading the pattern (richer but noisier)? Roadmap gap #3 lists
   this as open. Phase 2 above punts — uses pure extraction first.
3. Run traces: local-only vs. opt-in sync? Compression changes this math —
   Hypernym makes opt-in sync cheap enough that it might be worth doing.

---

## Non-goals for this spike

- Replacing TinyHumans Neocortex.
- Touching the hot inference loop in `src/openhuman/agent/`.
- Building general-purpose memory infra.
- Shipping anything before v1.

---

_Drafted 2026-04-23. Depends on Environment Contract roadmap gaps #1 and #2._
