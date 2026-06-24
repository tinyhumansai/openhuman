# Agent prompt audit: grounding, tools, skills, MCPs

_Audit + remediation of the orchestrator and sub-agent system prompts, focused on
anti-hallucination discipline and de-duplication. Inspired by
[NousResearch/hermes-agent](https://github.com/NousResearch/hermes-agent)'s named,
reusable guidance blocks._

## Why this exists

The system prompts for the orchestrator and the ~27 sub-agents had accreted "slop":
the same anti-fabrication paragraph copy-pasted (and quietly drifting) across many
agents, specialist walkthroughs living in the wrong agent, a `## Safety` block that said
nothing about grounding, and an em-dash rule restated inline despite a global suffix
already banning it. There was **no single source of truth** for how an agent should treat
its tool list, skills, or missing context, so each agent re-invented it.

## How the prompt system assembles (reference)

- `SystemPromptBuilder` (`src/openhuman/agent/prompts/builder.rs`) renders an ordered
  `Vec<Box<dyn PromptSection>>`, joins with `\n\n`, and appends `GLOBAL_STYLE_SUFFIX`.
- Concrete sections live in `src/openhuman/agent/prompts/sections.rs`.
- Three render paths produce a final system prompt:
  1. **Static chain** (`SystemPromptBuilder::with_defaults` / `for_subagent`).
  2. **Dynamic builders** (`from_dynamic`) — the orchestrator and ~25 other
     `agents/<id>/prompt.rs` files each hand-assemble their body by calling the free
     `render_*` helpers in `render_helpers.rs`. They funnel through
     `SystemPromptBuilder::build()` for the final wrap.
  3. **Narrow index-based renderer** (`render_subagent_system_prompt_with_format`) used by
     the sub-agent runner for spawned sub-agents (does NOT go through `build()`).
- **Skills**: the orchestrator lists installed skills by name+desc; full SKILL.md bodies
  are injected at *turn time* by `src/openhuman/workflows/inject.rs`, not in the system
  prompt.
- **MCPs**: not injected into any agent prompt. OpenHuman's MCP server only exposes tools
  to *external* clients; agents act through their own tool surface. There is no
  agent-facing MCP catalogue section, by design.
- **KV-cache contract**: the system prompt is built once per session and frozen. Anything
  in the cache-friendly prefix must be byte-stable (no time / RNG / host).

## Findings

| # | Finding | Resolution |
|---|---------|-----------|
| 1 | Anti-fabrication paragraph ("never invent ids; a tool not in your list does not exist") copy-pasted across crypto / markets / integrations / account-admin / mcp-setup / morning-briefing / researcher with no shared source. | Centralized into `GROUNDING_BODY` (one source of truth). |
| 2 | `## Safety` covered exfiltration / destructive commands but said nothing about grounding, not inventing tools/ids/files, or not ending a turn with a promise instead of an action. | Added the orthogonal grounding contract; kept `## Safety` for security. |
| 3 | Orchestrator `prompt.md` (235 lines) carried specialist content: a full Apple-Music click-sequence + keyboard walkthrough, and a ~47-line cron JSON walkthrough. | Moved to `desktop_control_agent` / `scheduler_agent`; orchestrator is now a lean routing table (175 lines). |
| 4 | Desktop-control instructions triple-duplicated across `SOUL.md`, orchestrator `prompt.md`, and `desktop_control_agent`. | `SOUL.md` + orchestrator trimmed to pointers; the worked example lives only in `desktop_control_agent`. |
| 5 | Em-dash rule restated inline in `orchestrator/prompt.md` although `GLOBAL_STYLE_SUFFIX` already bans em-dashes in every output. | Removed the inline restatement. |
| 6 | `omit_skills_catalog` rendering effect is inert (skills are agent-owned; the narrow renderer no longer emits a skills catalog). | **Retained.** See "Decisions" — the field is still live API surface (serde, CLI, 30+ TOMLs); removing it is a 60-site churn with no functional benefit. Documented as inert-but-retained. |

## The grounding contract

`GROUNDING_BODY` (`src/openhuman/agent/prompts/sections.rs`) is the canonical block.
It is deliberately **generic**: every clause must be true for *every* agent, including the
integrations executor. Agent-specific routing ("delegate external services", "pull slugs
from `composio_list_tools`") stays in that agent's own `prompt.md`.

```
## Grounding and tool use

- Your tools are exactly the ones listed in this prompt. You can only act through them.
  If a capability is not one of your tools, say so plainly rather than pretending it exists.
- Never invent tool names, arguments, ids, slugs, file paths, URLs, chain ids, addresses,
  quotes, metrics, or any other value. If you do not have it from a tool result or the user,
  ask for it or look it up with a tool.
- Use your tools to act. Do not just describe what you would do and stop, and never end a
  turn with a promise of future action: do it now, or hand back a concrete result.
- Never substitute plausible looking but fabricated output (made up data, invented file
  contents, synthesised tool or API responses) for results you could not actually produce.
  If a step failed, say it failed.
- Ground every factual claim in evidence you actually observed: a tool result, the user's
  message, or cited memory. If the evidence is missing, partial, or truncated, say so or
  fetch more instead of guessing.
- Skills run only via `run_workflow`, and only the skills listed as installed exist. Do not
  invent skill ids.
```

The first, third, and fourth bullets mirror Hermes's `TOOL_USE_ENFORCEMENT_GUIDANCE`
("use tools to act, never end a turn with a promise") and `TASK_COMPLETION_GUIDANCE`
("never substitute fabricated output for results you couldn't produce").

### How it reaches every agent

Rather than splice it into all ~26 dynamic `prompt.rs` files (drift-prone), the contract
is appended at the two chokepoints that every prompt funnels through, just like
`GLOBAL_STYLE_SUFFIX`:

- `SystemPromptBuilder::build()` — covers the static chain and all dynamic agents.
- `render_subagent_system_prompt_with_format()` — covers spawned sub-agents.

Both reference the same `GROUNDING_BODY` const, so they can never drift. A `render_grounding()`
helper and a `GroundingSection` PromptSection are also exposed for explicit use. Placement is
near the tail (just before the output-style rules) so it reads as a closing contract; the
text is byte-stable, so it stays in the cache-friendly prefix.

Coverage is asserted by `grounding_contract_appended_to_every_build_path`
(`mod_tests.rs`), which checks the marker appears exactly once across the static, sub-agent,
and dynamic build paths, plus an assertion in the narrow-renderer test.

## Decisions / deviations from the original plan

- **No `omit_grounding` flag.** The contract is always on. It is universal
  anti-hallucination guidance and the text is generic enough to be true for every agent, so
  threading an opt-out through serde + every caller would be churn for zero benefit.
- **`omit_skills_catalog` retained, not removed.** Its rendering effect is inert, but the
  field is wired into serde defaults, the agent CLI dump, and 30+ `agent.toml` files.
  Removing it is a 60-site change with real regression surface and no functional payoff,
  which is out of proportion for a prompt audit. Flagged here instead.
- **Cron delegate-vs-direct contradiction left in place.** The orchestrator decision tree
  routes scheduling to `schedule_task`, while the (now compressed) scheduling rules still
  let it call `cron_add` directly when that tool is in its list. Resolving which is
  authoritative is a behavioral change beyond this audit; the load-bearing rules
  (never via `run_workflow`; always confirm) were preserved.

## Touched files

- `src/openhuman/agent/prompts/sections.rs` — `GROUNDING_BODY` + `GroundingSection`.
- `src/openhuman/agent/prompts/builder.rs` — central append in `build()`.
- `src/openhuman/agent/prompts/render_helpers.rs` — `render_grounding()` + narrow-renderer append.
- `src/openhuman/agent/prompts/mod.rs` / `mod_tests.rs` — re-export + coverage tests.
- `src/openhuman/agent_registry/agents/{crypto_agent,markets_agent}/prompt.md` — drop generic clause.
- `src/openhuman/agent_registry/agents/orchestrator/prompt.md` — slimmed routing table.
- `src/openhuman/agent_registry/agents/{desktop_control_agent,scheduler_agent}/prompt.md` — received the moved walkthroughs.
- `src/openhuman/agent/prompts/SOUL.md` — trimmed duplicated desktop sequence.

## Verifying

```bash
GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml --lib agent::prompts::
GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml --lib agent_registry::agents
```

---

## Provider prompt-cache behaviour (#3939)

The byte-stable prompt prefix above is what makes KV-cache reuse possible. How
much of that reuse a user actually gets depends on the backend they route to.
`Provider::prompt_cache_capabilities()` (`src/openhuman/inference/provider/traits.rs`,
`PromptCacheCapabilities`) makes that contract explicit per provider:

| Provider | automatic prefix cache | reports cached input tokens | explicit cache-control | thread/session grouping |
|---|---|---|---|---|
| OpenHuman backend | ✓ | ✓ | — | ✓ (`thread_id` extension) |
| OpenAI / OpenRouter / GMI (OpenAI-compatible) | ✓ | ✓ | — | — (prefix identity) |
| Other / custom / LM Studio compatible | conservative default — all `false` | | | |

Notes:

- **Conservative by default.** Unknown or custom OpenAI-compatible slugs report
  no caching, so we never assume cache hits or send cache-only request fields a
  provider may not honour. Opting a provider in is a one-line edit to
  `prompt_cache_for_compatible_slug` (`compatible.rs`) once verified.
- **Usage normalization is provider-agnostic.** `extract_usage` already folds the
  OpenAI `usage.prompt_tokens_details.cached_tokens` shape and the
  `openhuman.usage.cached_input_tokens` extension into
  `UsageInfo.cached_input_tokens`, so cached-prefix cost accounting
  (`src/openhuman/agent/cost.rs`) is exact wherever the provider reports it.
- **No OpenHuman-only leakage.** `explicit_cache_control` stays `false` for every
  OpenAI-compatible provider (the chat-completions API has no such field), and
  `thread_id` grouping is declared only on `OpenHumanBackendProvider`.

Follow-ups (not in this slice): explicit cache-control request shaping for
providers that support it (e.g. Anthropic `cache_control`), a `ChatRequest`
cache-boundary marker, and extending cached-token parsing to non-OpenAI usage
shapes (e.g. DeepSeek `prompt_cache_hit_tokens`).
