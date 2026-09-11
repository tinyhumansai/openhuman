# Prompt / token-budget audit and plan

Date: 2026-08-31
Reference implementation studied: [nousresearch/hermes-agent](https://github.com/NousResearch/hermes-agent)

## Measured baseline

Numbers from `openhuman-core agent dump-all` run against the signed-in
workspace on this machine (`~/.openhuman/users/…/workspace`), 2026-08-31.
Token figures are bytes/4.

| Agent | Prompt text | Tool schemas (as dumped) | Tools |
| --- | --- | --- | --- |
| orchestrator | 37,703 B (~9,400 tok) | 175,476 B (~43,900 tok) | 220 |
| workflow_builder | 80,353 B (~20,100 tok) | — | — |
| integrations_agent@gmail | 23,667 B (~5,900 tok) | — | 25 |
| median specialist | ~7,000 B (~1,750 tok) | — | 2–6 (declared belt) |

Applying the default toolpack posture (`GroupMode::Withheld` for every
group) to the orchestrator's dumped catalogue:

```
220 tools                 175,476 B  ~43,869 tok
  - workflows   32 tools   40,803 B  ~10,200 tok
  - system      19 tools    5,058 B   ~1,264 tok
  - integrations 11 tools    5,484 B   ~1,371 tok
  - skills       9 tools    5,381 B   ~1,345 tok
  - composio     6 tools    3,867 B     ~966 tok
  - goals        3 tools    1,254 B     ~313 tok
  - crypto       1 tool     1,605 B     ~401 tok
= 139 tools advertised    112,024 B  ~28,006 tok
```

**The orchestrator's fixed per-turn prefix is roughly 37,400 tokens
before the user has typed anything** — ~28k of tool schema and ~9.4k of
prompt text. The prompt text is the smaller half of the problem.

Prompt-text breakdown (orchestrator, largest first):

| Section | ~tokens |
| --- | --- |
| Delegation (direct-first) | 1,978 |
| Rules | 1,325 |
| Installed Skills | 1,078 |
| …Running several workers at once | 632 |
| Connected Integrations | 551 |
| Writing style | 475 |
| …Capability questions about connected toolkits | 430 |
| Grounding and tool use | 399 |
| When OpenHuman is criticized | 268 |
| everything else (19 sections) | ~2,200 |

Single most expensive tool schemas: `propose_workflow` 1,892 tok,
`spawn_subagent` 909, `cron_add` 790, `memory_tree` 781,
`edit_workflow` 700, `suggest_workflows` 634. The top ten tools are
~6,500 tokens — more than every prose section except Delegation.

---

## Findings

### F1 — There is no prompt caching anywhere in the stack. (critical)

`grep -rn cache_control` returns **zero hits** across `src/`,
`vendor/tinyagents`, and `backend/src`. Every turn re-pays full input
price on the whole ~37k prefix.

Hermes treats this as the central design constraint. `agent/system_prompt.py`
builds the prompt as three ordered cache tiers — `stable` / `context` /
`volatile` — and its docstring states the rule plainly: *"Hermes never
re-renders parts of this string mid-session — that's the only way to keep
upstream prompt caches warm across turns."* It backs that with
`agent/prompt_caching.py`, `prompt_cache_boundary.py` (builder-declared
stable prefixes, so a webhook/cron scaffold gets a breakpoint at the exact
byte where the volatile tail starts) and `prompt_cache_scope.py`.

OpenHuman already does the hard half: `session/turn/core.rs:241` builds the
system prompt once on turn 1 and reuses it verbatim thereafter, with a
comment about preserving the KV prefix. What is missing is telling the
provider about it.

**This is the highest-value item on the list by a wide margin** — roughly a
90% discount on ~37k tokens of every turn on Anthropic-family models, for
no behavioural change.

### F2 — Tool schemas are 3× the prompt text, and nothing measures them.

Everything in the repo that talks about prompt size talks about prose. The
schemas are the actual budget. `propose_workflow` at 1,892 tokens is larger
than any single prose section; nobody would ship a 1,892-token prose block
without noticing.

### F3 — The dumper does not model the wire, so the budget is invisible.

`src/openhuman/agent/debug/mod.rs::render_via_session` dumps `agent.tools()`
— the whole registry — and passes `empty_visible` as the visible set. It
never calls `strip_packed_from_visible` and never applies the agent's
declared `[tools] named` belt. Consequences:

- Every specialist reports `tools=197` / 47k tok in `SUMMARY.txt`. The real
  belts are 2–6 tools (`researcher` = `web_search_tool`, `web_fetch`;
  `critic` = `read_diff`, `run_linter`, `run_tests`, `file_read`).
- The orchestrator's 220 overstates its real 139.
- There is no per-section or per-tool byte breakdown at all.

Hermes ships `hermes prompt-size` (`hermes_cli/prompt_size.py`, with
`--json`): it builds a real offline agent with dummy credentials so the
numbers match the wire, then reports system-prompt total, the
`<available_skills>` index, memory + profile, and tool-schema JSON, plus a
per-skill table. Its module docstring names the goal: *"Lets users see where
their fixed prompt budget goes … without parsing a saved session JSON by
hand."*

### F4 — Toolpack withholding is all-or-nothing; there is no middle setting.

`GroupMode` is `Advertised` / `Withheld` / `Off`. Withheld removes the
schema entirely and substitutes a `load_skill` round trip. There is no way
to say "keep the name, drop the description and parameter schema", which is
the cheap 80% for a tool the model needs to *know exists* but rarely calls.

Hermes does exactly this for its skills index: a category outside the
current posture renders as one `category [names only]: a, b, c` line, and
the code carries an explicit warning not to remove entries entirely —
*"agent-created skills are the model's project memory, and models don't
reach for skills_list to rediscover what the index stops showing them."*

### F5 — Section order puts volatile content in the cache prefix.

`SystemPromptBuilder::with_defaults()` orders: Identity, **UserFiles
(PROFILE.md, MEMORY.md)**, AgentsInstructions, **UserMemory**, Tools,
Safety, Workspace, DateTime, Runtime. The two most volatile inputs in the
whole prompt sit at positions 2 and 4, ahead of every byte-stable block.
Any memory write invalidates the entire remainder of the prefix.

Hermes puts precisely these in the volatile tier: *"volatile — skills index,
memory snapshot, user profile, external memory provider block, timestamp
line"*, rendered last, with the ordering rationale spelled out for both
explicit-breakpoint and longest-prefix backends.

Smaller instance of the same bug in the current render: `## Workspace`
(~185 tok) and `# Writing style` (~475 tok) are both byte-stable but are
emitted *after* `## Current Date & Time`.

### F6 — Flat character caps, no budget.

`BOOTSTRAP_MAX_CHARS = 20_000` applies per injected file, and both the
global and the project `AGENTS.md` get one — up to ~10k tokens of project
instructions alone. `USER_FILE_MAX_CHARS = 2_000`. There is no total
budget, no scaling to the resolved context window, and no test asserting a
ceiling on the assembled prompt.

Hermes resolves the model's context window once per session and scales its
context-file caps to it (`_dynamic_context_file_max_chars`), and surfaces
truncation as a user-visible status message rather than a log line.

### F7 — Prose bloat is real but second-order.

`workflow_builder` renders an 80KB / ~20k-token system prompt — twice the
orchestrator's. Delegation (1,978) + Rules (1,325) + the two `###`
subsections under them (1,062) are ~4.4k tokens of the orchestrator's 9.4k.
Worth trimming, but it is ~3k of a 37k problem. Do it after F1–F5.

---

## The structural problem: everything is disclosed up front

F1–F7 are byte-level. Underneath them is one architectural choice: **every
tool OpenHuman registers is advertised**. There is no per-tool notion of
"registered but not on the wire". The toolpack system is the only lever, it
is per-*group* rather than per-tool, it is a config posture rather than a
property of the tool, and its recovery path is a `load_skill` round trip
rather than a search.

Both reference implementations treat disclosure as the primary axis.

### Codex: exposure is a property of the tool

`codex-rs/tools/src/tool_executor.rs` defines `ToolExposure` per tool:

```rust
pub enum ToolExposure {
    Direct,             // in the initial model-visible list
    Deferred,           // registered, omitted from the list, found via tool search
    DeferredModelOnly,
    DirectModelOnly,
    CodeModeOnly,       // only callable from nested Code Mode scripts
    Hidden,             // registered for dispatch, never shown
}
```

`tools/handlers/tool_search.rs` builds a **BM25 index** over every entry where
`tool.exposure.is_deferred()` and exposes one `tool_search` tool. The model
searches for a capability, gets the schema back, and calls it. A deferred tool
costs zero tokens until it is needed.

Codex goes one step further with **Code Mode**
(`codex-rs/core/src/tools/code_mode/`): tools are callable from inside a
nested script rather than as individual function schemas, so one tool replaces
an entire namespace on the wire.

OpenHuman already has the machinery — `search_tool_catalog` and
`get_tool_contract` — but they are pointed at the **Composio action
catalogue**, not at its own registry, and they are gated behind `flows`.

### Hermes: capabilities live in skills, and tools collapse to verbs

Hermes' full-fat coding profile is **31 tools**. OpenHuman's orchestrator
advertises **139**. The difference is not that Hermes does less; it is
where the surface lives.

- Its entire skill system is **three tools** — `skills_list`, `skill_view`,
  `skill_manage` — plus a one-line-per-skill index. Everything a skill can do
  is behind `skill_view`.
- Tool families collapse to a single tool with an action argument:
  `memory` is one tool, `cronjob` is one tool, `todo` is one tool,
  `discord` is one tool, `spotify_playback` covers play/pause/skip.

Measured against OpenHuman's advertised set:

| Family | OpenHuman | ~tokens | Hermes | Collapsed cost |
| --- | --- | --- | --- | --- |
| memory | 12 tools | 2,831 | 1 (`memory`) | ~780 |
| todo | 6 tools | 1,375 | 1 (`todo`) | ~450 |
| cron | 6 tools | 1,040 | 1 (`cronjob`) | ~790 |
| task | 6 tools | 632 | — | ~200 |
| spawn | 3 tools | 1,895 | 1 (`delegate_task`) | ~910 |
| delegate | 3 tools | 900 | — | ~380 |
| **total** | **36 tools** | **8,673** | **6 tools** | **~3,510** |

That is ~5,200 tokens from arity alone, with no capability removed.

### What deferring buys on top

Ranked by schema size, the orchestrator's advertised set concentrates hard:

```
top 20 tools    8,638 tok     tail 119 tools   19,367 tok
top 30 tools   11,585 tok     tail 109 tools   16,420 tok
top 50 tools   16,870 tok     tail  89 tools   11,135 tok
```

Keeping ~25 tools `Direct` and deferring the rest behind one `tool_search`
removes roughly **16,000 tokens** from every orchestrator turn.

### Where the workflow surface should go

The `workflows` pack is 32 tools and 10,200 tokens — the single largest line
item in the catalogue, and larger than the orchestrator's entire system
prompt. On the orchestrator it is already withheld, so it costs nothing
there today; the cost lands on `workflow_builder` and `flow_discovery`, which
*own* the pack and therefore get all 32 advertised **plus** a 20,100-token
system prompt. That agent pays over 30k tokens of fixed prefix to build a
flow.

A skill is the right container for it, and the arithmetic is stark: a skill
costs one line in the index. OpenHuman's `## Installed Skills` block is 1,078
tokens for 19 skills — **~57 tokens per capability** against ~319 tokens per
tool. The flow DSL reference currently living in `workflow_builder/prompt.md`
is exactly what a `SKILL.md` body is for: loaded when the agent is building a
flow, absent otherwise.

The same argument applies to `crypto` (20 tools), `documents`, `audio`, and
`app_update`. The toolpack table is already a list of candidates — it was
built as a compression mechanism, and a skill is the same mechanism with a
body attached and no bespoke `load_skill` protocol.

---

## Plan

### P0 — Make the budget visible (prerequisite)

1. Fix `render_via_session` to model the wire: apply the definition's
   `ToolScope` belt and call `strip_packed_from_visible(&mut visible,
   agent_id)` before collecting specs. The `.tools.json` must be the
   advertised set.
2. Add `openhuman-core agent prompt-size [--agent <id>] [--json]`, modelled
   on `hermes_cli/prompt_size.py`: total, per-section bytes/tokens, tool
   schemas with a per-tool table, skills index, memory + profile, AGENTS.md
   layers.
3. Add `scripts/prompt-budget.limits` + a ratchet lane, same shape as
   `scripts/kernel-floor.limits` / `check-kernel-floor.sh` — fail on growth,
   fail on an unratcheted improvement.

*Exit: `SUMMARY.txt` shows `researcher tools=2`; CI fails if the orchestrator
prefix grows.*

### P1 — Prompt caching

4. Split `build_system_prompt` into three tiers mirroring Hermes'
   `build_system_prompt_parts`: `stable`, `context`, `volatile`. Join for the
   wire; keep the tier boundaries as offsets.
5. Emit `cache_control: {type: "ephemeral"}` at the tier boundaries for
   Anthropic-family providers, with the tool array ahead of the system block
   so schemas fall inside the cached prefix. Same on the backend proxy.
6. Reorder `SystemPromptBuilder::with_defaults()` to match the tiers —
   `UserFilesSection` and `UserMemorySection` to the tail after
   `DateTimeSection`; workspace and style ahead of it.

*~37k tokens/turn at roughly 10% of list price on a hit.*

### P2 — Disclosure architecture (the structural fix)

7. **Per-tool exposure.** Add `Tool::exposure() -> ToolExposure` with
   `Direct` / `Deferred` / `Hidden`, defaulting to `Direct` so nothing moves
   until it is opted in. This replaces the per-group `GroupMode` as the
   primary axis; `ToolGroups` stays as the embedder-facing override.
8. **Tool search over our own registry.** Add `tool_search` backed by a BM25
   index over deferred tools (Codex uses the `bm25` crate for exactly this).
   Ungate it from `flows` — the existing `search_tool_catalog` /
   `get_tool_contract` pair stays pointed at Composio.
9. **Defer the long tail.** Keep ~25 tools `Direct`; mark the rest
   `Deferred`. Drive the choice from Langfuse call-frequency data, not
   intuition. *(~16,000 tok)*
10. **Collapse tool families to verbs.** `memory_*` (12 → 1), `todo_*` (6 → 1),
    `cron_*` (6 → 1), `task_*` (6 → 1), `spawn_*` + `delegate_*` (6 → 2).
    Keep the old names as dispatch aliases for one release. *(~5,200 tok)*
11. **Move whole capability surfaces into skills**, starting with
    `workflows` — 32 tools and 10,200 tokens, plus the flow DSL reference
    now inlined in `workflow_builder/prompt.md`. Then `crypto`, `documents`,
    `audio`, `app_update`. A capability costs ~57 tokens as a skill entry
    against ~319 as a tool.
12. Add a per-tool schema budget to the P0 ratchet — warn above 400 tokens,
    fail above 800. Rewrite the six offenders; `propose_workflow` (1,892) first.

*Target: 28,006 → roughly 6,000 tokens of advertised schema.*

### P3 — Budget-aware injection

13. Replace the flat `BOOTSTRAP_MAX_CHARS` with a cap scaled to the resolved
    context window, and give the combined injected-file set one budget rather
    than one cap each.
14. Surface truncation to the user as a status event, not only a log line.

### P4 — Prose trim

15. `workflow_builder` (~20k tok) — most of it becomes the `workflows`
    SKILL.md body in P2/11; trim what remains.
16. Rewrite Delegation and Rules on the orchestrator.

### P5 — Horizon: Code Mode

17. Evaluate Codex's `code_mode` seam — tools callable from a nested script
    instead of as individual function schemas. It is the endgame for schema
    cost, and it composes with P2 rather than replacing it. Not before P0–P2
    land and the ratchet has real numbers behind it.

## Sequencing note

P0 before everything: three findings (F2, F3, F4) exist only because the
numbers were never on screen. Then P1, which is a pricing change with no
behavioural risk. P2 is the largest and the riskiest — it changes what the
model can see — so it wants the measurement and the caching in place first,
and it should land tool family by tool family behind the ratchet.
