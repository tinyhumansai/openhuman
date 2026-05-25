---
description: >-
  One subscription, many models. Tasks pick their model via hint prefixes:
  reasoning goes to a strong model, fast paths go to a fast one, vision to vision.
icon: route
---

# Automatic Model Routing

Different parts of an agent want different models. Long reasoning wants a frontier model. Quick "fix this typo" calls want a fast cheap one. Vision wants a vision model. OpenHuman handles this with a built-in **router provider** so you never have to think about it.

## How a request gets routed

The model parameter on any chat call can take one of two shapes:

- **Concrete model name**. e.g. `anthropic/claude-sonnet-4`. Routes to the default provider with that exact model.
- **Hint prefix**. e.g. `hint:reasoning`. Looks the hint up in the route table and resolves to a `(provider, model)` pair.

```rust
// src/openhuman/providers/router.rs
fn resolve(&self, model: &str) -> (usize, String) {
    if let Some(hint) = model.strip_prefix("hint:") {
        if let Some((idx, resolved_model)) = self.routes.get(hint) {
            return (*idx, resolved_model.clone());
        }
    }
    (self.default_index, model.to_string())
}
```

The router wraps several pre-created providers (Anthropic, OpenAI, Google, Groq, etc.) and picks the right one per request. Hints can be remapped at runtime without restarting the core.

## Common hints

| Hint | Typical target | When it's used |
| --- | --- | --- |
| `hint:reasoning` | A strong reasoning model | Multi-step planning, math, code-heavy turns |
| `hint:fast` | A fast/cheap model | UI helpers, autocompletes, small classification calls |
| `hint:vision` | A vision-capable model | Screenshots, image attachments, OCR |
| `hint:summarize` | A model good at compression | Memory tree summary builders |
| `hint:code` | A code-tuned model | Native coder turns |

The exact mappings are configurable; the defaults ship sensible per-provider routes.

## One subscription

Routing happens behind a single OpenHuman subscription. You don't hold separate API keys for Anthropic, OpenAI, Google etc., the backend brokers access, and the router picks the right one per task. That's the "one subscription, many providers" promise from the README, made concrete.

## Overriding routes

- **Globally**. config TOML (`Config` struct in `src/openhuman/config/schema/types.rs`) can supply a custom route table at startup.
- **Per call**. pass a concrete model name (no `hint:` prefix) and the router falls through to the default provider with that exact model.
- **For a skill**. skills can pin a hint or a model in their manifest.

## Per-agent model pins

Sub-agents can also pin an exact model without disabling automatic routing for the rest of the app. Use this when an orchestrator or team lead needs a stronger model, while high-volume leaf agents should stay on a cheaper one.

Inline calls win for one delegation:

```json
{
  "agent_id": "researcher",
  "model": "anthropic/claude-sonnet-4",
  "prompt": "Collect source notes for the launch memo."
}
```

Persistent defaults live in `config.toml`:

```toml
[orchestrator]
model = "anthropic/claude-sonnet-4"

[teams.research]
lead_model = "openai/gpt-5.1"
agent_model = "groq/llama-3.1-8b-instant"

[teams.code]
agent_model = "qwen/qwen3-coder"
```

Resolution order:

1. Inline `model` on `spawn_subagent` or an archetype delegation call.
2. `[orchestrator].model` or `[teams.<team>]` / built-in aliases such as `[teams.research]` and `[teams.code]`.
3. The archetype's own model hint and the normal route table.

For `[teams.*]`, `lead_model` applies to agents that can delegate and `agent_model` applies to leaf workers. If only one is set, the harness falls back to it for both roles.

## Why this isn't just "model switcher"

Routing isn't a UI dropdown. The agent loop itself emits hints based on what it's about to do. You don't pick the model; the *task* does. That's the difference between "multi-model" and "smart routing".

## See also

- [Smart Token Compression](../token-compression.md). what makes large reasoning calls affordable.
- [Native Tools](../native-tools/README.md). different tool calls hint at different routes.
- [Local AI (optional)](local-ai.md). lightweight chat hints can run on-device.
