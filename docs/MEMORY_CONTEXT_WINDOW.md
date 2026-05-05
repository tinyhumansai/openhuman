# Long-term memory window

User-facing setting that controls how much long-term memory OpenHuman injects
into every new agent / orchestrator session.

## What it changes

Two distinct injection paths share one preset so the user only has to make one
choice:

1. **Recalled memory + working memory** — the `[Memory context]` and
   `[User working memory]` blocks built by
   [`DefaultMemoryLoader::load_context`](../src/openhuman/agent/memory_loader.rs)
   on every turn.
2. **Tree-summarizer root summaries** — the per-namespace root summaries
   pulled into the system prompt on the first turn of a session by
   [`fetch_learned_context`](../src/openhuman/agent/harness/session/turn.rs).

Both call sites read the active limits from
[`AgentConfig::resolved_memory_limits`](../src/openhuman/config/schema/agent.rs).

## Presets

| Preset | Recall cap (chars) | Per-namespace tree cap | Total tree cap |
|---|---:|---:|---:|
| `minimal` | 800 | 2 000 | 8 000 |
| `balanced` *(default)* | 2 000 | 8 000 | 32 000 |
| `extended` | 4 000 | 16 000 | 64 000 |
| `maximum` | 8 000 | 32 000 | 128 000 |

`balanced` matches the historical hard-coded behaviour. `maximum` is bounded so
prompts cannot grow beyond ~32k tokens of injected long-term memory regardless
of how many namespaces a workspace accumulates.

## Where the setting lives

- **Storage**: `agent.memory_window` in the persisted config TOML.
- **Read**: `openhuman.get_config` → `config.agent.memory_window`.
- **Write**: `openhuman.update_memory_settings` with
  `{ "memory_window": "minimal" | "balanced" | "extended" | "maximum" }`.
- **UI**: `Settings → Memory Data → Long-term memory window`
  (`app/src/components/settings/components/MemoryWindowControl.tsx`).

## Design rules

- **Core owns the budgets.** The frontend stores a label only; mapping
  label → char caps lives in
  [`MemoryContextWindow::limits`](../src/openhuman/config/schema/agent.rs).
  A buggy or future client cannot pick "infinite memory" by accident.
- **Stepped, not freeform.** The presets are deliberately discrete so the UX
  copy (`Minimal` / `Balanced` / `Extended` / `Maximum`) and the actual
  budgets line up. There is no raw "memory budget" slider in the UI.
- **Backward-compat raw override (unmigrated configs only).** A config from
  before this setting existed deserializes with `memory_window = None`. While
  unmigrated, the resolver falls back to the legacy
  `agent.max_memory_context_chars` for the recall cap (clamped to the
  `Maximum` preset's ceiling). The first time a preset is written — by the UI
  or by any client — `memory_window` becomes `Some(...)` and the preset is
  authoritative; the legacy raw field is then ignored entirely. This means
  picking `Minimal` in the UI on a config that previously had a wider raw
  value really does shrink injection size.
- **Safety bound.** The `maximum` preset is the absolute ceiling. No code path
  in the harness reads memory caps from anywhere other than
  `resolved_memory_limits`, so this ceiling is the single fact to audit.

## Adding a new preset

1. Extend `MemoryContextWindow` in `src/openhuman/config/schema/agent.rs` and
   add its limits in `MemoryContextWindow::limits`.
2. Update `as_str` / `from_str_opt` so the RPC + config TOML round-trip works.
3. Add the label to `MEMORY_CONTEXT_WINDOWS` and the meta map in
   `app/src/components/settings/components/MemoryWindowControl.tsx`.
4. Add unit tests in both `memory_window_tests` (Rust) and
   `MemoryWindowControl.test.tsx` (Vitest).
