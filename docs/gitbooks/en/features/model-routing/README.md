---
description: >-
  One subscription, many models. Each task picks its model with a hint:
  reasoning goes to a strong model, quick work to a fast one, images to a
  vision model.
icon: route
---

# Automatic model routing

Different parts of an agent want different models. Long reasoning wants a frontier model. A quick "fix this typo" wants a fast, cheap one. Images want a vision model. OpenHuman has a built-in router that picks for you.

## How a request is routed

The model parameter on a chat call takes one of two forms:

- **A concrete model name**, such as `anthropic/claude-sonnet-4`. It goes to the default provider with that exact model.
- **A hint prefix**, such as `hint:reasoning`. The router looks the hint up in the route table and resolves it to a provider and model.

That is the whole rule. The router strips a `hint:` prefix, looks up the rest in the route table, and falls through to the default provider with the name unchanged if there is no entry. A name without the prefix is never rewritten.

The route table is `[[model_routes]]` in `config.toml`, with one entry per hint. `[[embedding_routes]]` does the same for embeddings. You can remap hints at runtime without restarting the core.

## Common hints

| Hint             | Typical target                    | When it is used                                       |
| ---------------- | --------------------------------- | ----------------------------------------------------- |
| `hint:reasoning` | A strong reasoning model          | Multi-step planning, math, code-heavy turns           |
| `hint:fast`      | A fast, cheap model               | UI helpers, autocomplete, small classification calls  |
| `hint:vision`    | A vision-capable model            | Screenshots, image attachments, OCR                   |
| `hint:summarize` | A model good at compression       | Background summarization                              |
| `hint:code`      | A code-tuned model                | Native coder turns                                    |
| `hint:burst`     | A high-throughput, low-cost model | Cheap, latency-tolerant work for high-fanout agents   |

The mappings are configurable, and the defaults ship with sensible per-provider routes.

## One subscription, or your own

By default, routing happens behind a single OpenHuman subscription. You don't hold separate API keys for Anthropic, OpenAI or Google. The backend brokers access and the router picks the right model for each task.

The subscription is a default, not a requirement. The same router works with your own provider key or a local model on a runtime you run yourself (Ollama, LM Studio, MLX, or any OpenAI-compatible server). You choose per workload and can mix all three. See [Local models and bring your own key](local-and-byok-models.md) for setup and for what each route supports for chat, vision and embeddings.

## Overriding routes

- **Globally:** `[[model_routes]]` in `config.toml` sets the route table at startup.
- **Per call:** pass a concrete model name with no `hint:` prefix. The router falls through to the default provider with that exact model.
- **For a skill:** a skill can pin a hint or a model in its manifest.
- **Per agent:** an agent definition can pin its own provider and model, which wins over the table.

## Default model

**Settings > Connections > LLM > Routing** has a **Default model** row. It sets a model from the managed catalog that every managed chat turn runs on, instead of the anonymous chat tier. It opens on DeepSeek V4 Flash. The model pill in the composer can still override it for one conversation. The specialized tiers (reasoning, coding, vision and summarization) keep their own routing. The rows below it send each workload to Managed, a BYOK provider, a local runtime or Claude Code.

## Per-agent model pins

Sub-agents can pin an exact model without turning off automatic routing for the rest of the app. This helps when an orchestrator or team lead needs a stronger model while high-volume leaf agents stay on a cheaper one.

An inline pin wins for a single delegation:

```json
{
  "agent_id": "presentation_agent",
  "model": "anthropic/claude-sonnet-4",
  "prompt": "Build a five-slide deck from the Q3 report."
}
```

Lasting defaults go in `config.toml`:

```toml
[orchestrator]
model = "anthropic/claude-sonnet-4"

[teams.planner]
lead_model = "openai/gpt-5.1"
agent_model = "groq/llama-3.1-8b-instant"

[teams.image]
agent_model = "openai/gpt-5.1"
```

The router picks the model in this order:

1. The inline `model` on `spawn_subagent` or an archetype delegation call.
2. `[orchestrator].model`, or `[teams.<agent_id>]`, or the alias without `_agent` (`[teams.image]` for `image_agent`).
3. The archetype's own model hint and the normal route table.

For `[teams.*]`, `lead_model` applies to agents that can delegate, and `agent_model` applies to leaf workers. If only one is set, it is used for both roles.

## Why this is more than a model switcher

Routing is not a dropdown. The agent loop itself picks a hint based on what it is about to do. You don't pick the model, the task does. That is the difference between multi-model and smart routing.

## See also

- [Token compression](../token-compression.md): what makes large reasoning calls affordable.
- [Native tools](../native-tools/README.md): different tool calls hint at different routes.
- [Local models and bring your own key](local-and-byok-models.md): run on your own key or fully on-device.
- [Local AI](local-ai.md): add your own local runtime as a provider. Lightweight chat hints can run on-device.
- [Pluggable engines](../../developing/engines.md): how config chooses the provider layers.
- [One TinyHumans API key](../../developing/tinyhumans-api-key.md): the single key behind managed routing.
