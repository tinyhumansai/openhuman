# Telegram (and all channels) honor `chat_provider` workload routing

**Issue:** [#3098](https://github.com/tinyhumansai/openhuman/issues/3098), sub-issue 1.
**Scope:** Sub-issue 1 only. Sub-issues 2 (Telegram file commits — by-design question), 3 (skills not running), and 4 (Brave Search) are out of scope.

## Problem

A user picks Ollama in **Settings → AI** (which writes `chat_provider = "ollama:<model>"` to config), but the Telegram bot still routes through the managed OpenHuman cloud backend and the user's `config.default_model`. The Ollama selection is silently ignored by every channel (Telegram, Discord, Slack, iMessage, Mattermost).

## Root cause

`src/openhuman/channels/runtime/startup.rs:171-183, 267-270, 682-695` builds the channels runtime provider via `create_intelligent_routing_provider(…)` — an exclusively cloud-backed chain (`OpenHumanBackendProvider` wrapped in `ReliableProvider` and the legacy hint-based `IntelligentRoutingProvider`). It pairs that provider with `ctx.model = config.default_model.unwrap_or(DEFAULT_MODEL)`.

It never consults `provider_for_role("chat", &config)` or `Config::workload_local_model("chat")` — the documented single source of truth for "is this workload local?" (`src/openhuman/config/schema/types.rs:511-544`). The unified workload factory `inference::provider::create_chat_provider` (`src/openhuman/inference/provider/factory.rs:206-217`) already knows how to build the right `(provider, model_id)` for any chat-workload string (`"ollama:…"`, `"lmstudio:…"`, `"<byok-slug>:…"`, `"openhuman"`, `"cloud"`). The channel runtime simply doesn't use it.

## Fix

In `runtime/startup.rs`, branch on `provider_for_role("chat", &config)` once during runtime setup:

- **Workload override path** (`chat_provider` is `"ollama:…"`, `"lmstudio:…"`, `"<byok-slug>:…"`, or `"claude_agent_sdk[:…]"`): build the provider via `inference::provider::create_chat_provider("chat", &config)`. Use the returned `model_id` as `ctx.model`. The cache key (`ctx.default_provider`) becomes the slug portion of the provider string (e.g. `"ollama"`).
- **Default path** (unset, `"cloud"`, or `"openhuman"`): keep the existing `create_intelligent_routing_provider` chain verbatim. `ctx.model` continues to come from `config.default_model`. Zero behavior change for cloud users.

The branch happens once during channel-runtime startup. All channels share `ChannelRuntimeContext`, so the fix uniformly covers Telegram, Discord, Slack, iMessage, Mattermost (and Matrix when feature-gated on).

## Behavior matrix

| `chat_provider` | Today | After fix |
|---|---|---|
| unset / `"cloud"` / `"openhuman"` | Cloud + `default_model` | Cloud + `default_model` (unchanged) |
| `"ollama:llama3.2"` | Cloud + `default_model` (bug) | Ollama + `llama3.2` |
| `"openai:gpt-4o"` (BYOK) | Cloud + `default_model` (bug) | OpenAI + `gpt-4o` |

## Tests

Unit-level coverage in the existing channels runtime test surface:

1. `chat_provider` unset → `ctx.model == config.default_model` (regression guard for cloud users).
2. `chat_provider = "ollama:llama3.2"` → `ctx.model == "llama3.2"` and `ctx.default_provider == "ollama"`.
3. `chat_provider = "cloud"` → identical to (1).

Tests use the existing `test_support.rs` harness and `Config` builders. No new mocks required; the workload factory is already exercised by `factory_tests.rs`.

## Non-goals (deliberate)

- **`/model <id>` per-conversation override** — `routes.rs:169-211`'s `get_or_create_provider` ignores the provider name and always rebuilds a cloud provider. This is a pre-existing limitation unrelated to this fix; addressing it would expand scope significantly. Once the default is correct, the `/model` command becomes a model-name-only override against whichever provider the channel runtime was constructed with, which is a reasonable interim state.
- Sub-issues 2, 3, 4 of #3098 — separate root causes, will each get their own PR if/when triaged.
- Any change to `local_ai.usage.*` (the deprecated legacy hint-based routing) — explicitly out of scope per the comment in `config/schema/types.rs:520-523`.

## Blast radius

- One file edited: `src/openhuman/channels/runtime/startup.rs` (~15 lines changed).
- Cloud-only users: zero behavior change (the default branch is the existing code path).
- Local-model users: gain a working Telegram/Discord/Slack/etc. experience with their selected Ollama (or BYOK) provider.
