---
description: >-
  The [components] config table: one switch per tiny dependency, how it narrows
  a host, and how legacy flags migrate.
icon: toggle-on
---

# Components

`config.toml` has a `[components]` table with one boolean per tiny dependency. All default to `true`.

```toml
[components]
flows = true       # tinyflows: saved workflows
tinyflows = true   # tinyflows module: scheduled jobs (cron) runtime, store and schedule maths
documents = true   # tinydocs
search = true      # tinysearch
mcp = true         # tinymcp
connectors = true  # tinyconnectors (Composio)
hosting = true     # tinyhosts
juice = true       # tinyjuice
memory = true      # tinymemory
wallet = true      # tinywallet (web3, x402)
computer = true    # tinycomputer
sandbox = true     # tinybox
channels = true    # tinychannels
skills = true      # tinyskills
voice = true       # tinyvoice
eventqueues = true # tinyeventqueues: inbound event queues
```

Each switch also reads `OPENHUMAN_COMPONENT_<NAME>=0|1` (for example `OPENHUMAN_COMPONENT_VOICE=0`); the environment wins over the file.

## What a switch does

The switches only narrow. The core resolves them into a component set and computes `effective = host choice ∩ components` for the domain families, background services and tool groups. A host preset (`harness`, `kernel`, `saas`, ...) decides what may exist; a switch can remove something from it, never add something back.

A method whose component is off answers `COMPONENT_DISABLED:<name>` (JSON-RPC `error.message`, with `error.data = {"kind": "component_disabled", "component": "<name>"}`) instead of `unknown method`, so the UI and CLI can explain why. A method that the host simply left out still looks unregistered.

Loadable modules follow the same switches. A module whose component is off is never downloaded, mapped or registered: `modules.list` reports its state as `disabled`, boot skips it (including a matching artifact on the module search path), and loading it returns `COMPONENT_DISABLED:<name>`. `about_app` marks capabilities backed by a disabled component with status `disabled`.

A flag change takes effect on the next core start. There is no hot toggling.

Each component's domain wires itself to its switch (tools, services and RPC) in its own change; until then a switch removes only the families, services and tool groups listed in the core's component set.

## Tinyflows

`tinyflows = false` stops the cron scheduler service and makes every `cron.*` RPC answer `COMPONENT_DISABLED:tinyflows`; the `tinyflows` module is never downloaded or loaded, so no job runs and none can be stored. It is separate from `flows`, which only stops schedule-triggered flows while agent, shell and system jobs keep running.

## Not toggleable

The kernel dependencies (tinyagents, tinytools, tinyinference, tinybus) are always on. `[modules] enabled` remains the global kill switch for loadable native modules.

## Legacy flags

The first load of an older config moves these once into `[components]`, and the next save writes the result:

| Legacy setting | Becomes |
| --- | --- |
| `[hosting] enabled = false` | `components.hosting = false` |
| `[search] enabled = false` | `components.search = false` |
| `[mcp_client] enabled = false` | `components.mcp = false` |
| `[memory] engine = "none"` | `components.memory = false` (`engine` is kept) |
| `OPENHUMAN_DISABLE_CHANNEL_LISTENERS=1` | `components.channels = false` at load (env override, kept for one release; `OPENHUMAN_COMPONENT_CHANNELS` wins) |
| `[dictation] enabled`, `[voice_server] auto_start` and `always_on_enabled` all explicitly `false` | `components.voice = false` (any other mix keeps voice on); these legacy keys are migration inputs only |
| `[sandbox] enabled = false` (or `[security.sandbox] enabled = false`) | `components.sandbox = false` (`enabled = true` or unset keeps it on; the old key is kept) |
| `[composio] enabled` | dropped; it was never read |

A `components.*` key already present in the file always wins over its legacy flag, so the migration is idempotent and turning a component back on sticks.

## Removed components

TinyMemes (final-reply slang and meme remixing) is no longer part of OpenHuman. A config that still has `components.memes` or a `[memes]` section loads normally; the keys are ignored and dropped on the next save. `OPENHUMAN_TINYMEMES*` and `OPENHUMAN_COMPONENT_MEMES` are no longer read.

## Event queues

`eventqueues` is the inbound event queues (the `tinyeventqueues` module), tuned by the `[inbound]` section. When it is on and `[inbound] enabled` is `true` (the default), Composio triggers and agent webhooks are no longer triaged where they arrive. They are ingested into the module, which writes them to the host's storage before answering. The module rejects a redelivery of the same trigger (the Composio trigger uuid, or the webhook's correlation id), filters and paces a flood (rate limits fold the overflow into one digest), and calls the host back with `Deliver` when each event is due. Triage runs then. A triage that comes back `Deferred` is retried at its `defer_until_ms` instead of being dropped. A failed or slow triage backs off, and an event that runs out of attempts, or that triage marks terminal, is parked in the dead-letter queue. `inbound.stats`, `inbound.dead` and `inbound.redrive` inspect and replay the queues. They answer `COMPONENT_DISABLED:eventqueues` when the switch is off.

With the switch off, `[inbound] enabled = false`, or the module unavailable on this host, events take the direct triage path as before. An event too large to queue (over 192 KiB encoded) also takes the direct path.

A delivery is acknowledged once triage has decided, and the decision is applied in its own task. A long agent run therefore never holds a queue lease, but an apply that fails is reported rather than retried. A deferral counts as a delivery attempt: a trigger still deferring after the lane's `max_attempts` (5) is dead-lettered and can be redriven. Webhooks are deduplicated per request, by correlation id. When the queues are active the module is loaded in the background at startup, so work a previous run left queued is delivered without waiting for the next event.

| `[inbound]` key | Default | Meaning |
| --- | --- | --- |
| `enabled` | `true` | route inbound events through the queues |
| `queues_path` | unset | a `queues.json` replacing the module's shipped one; the host declares exactly these queues on the module's storage scope |
| `pipeline_path` | unset | a `pipeline.json` replacing the shipped one (gate, coalescing, salience, lanes, cadences, retry back-off) |
| `queue_url` | `storage` | where the queues live: the module's storage scope, or `amqp://` / `amqps://` for a RabbitMQ broker in builds with the storage `rabbitmq` driver |
| `triage_timeout_ms` | `25000` | how long one delivered event's triage may take before the delivery is retried; at most 25 s, below the module's 30 s callback deadline; read at startup |

### External sources and pinned agents

An integration outside the core (an X bot, a feed reader) can push its own events into the same queues and have each batch handled by one agent it names, with no triage. Configure the source under `[inbound.sources."<name>"]`; the name is `<kind>:<name>` (`^[a-z0-9_-]+:[a-z0-9_.*-]+$`), and the prefixes `composio:`, `webhook:`, `web:` and `channel:` are reserved for the core's own paths.

```toml
[inbound.sources."x:mentions"]
agent_id = "teeny-mentions"   # required: a built-in or workspace agents/*.toml agent
debounce_ms = 30000
max_wait_ms = 300000
max_batch = 20
prompt = "Draft a reply to each mention.\n{{events}}"
result_ttl_days = 7
```

| `[inbound.sources.<name>]` key | Default | Meaning |
| --- | --- | --- |
| `agent_id` | required | the agent every batch runs on |
| `debounce_ms` | `5000` | flush a batch this long after its last event |
| `max_wait_ms` | `60000` | flush a batch this long after its first event at the latest |
| `max_batch` | `25` | the most events one turn carries; a full batch flushes at once |
| `prompt` | a short instruction | the turn's message; `{{events}}` is replaced with the batch as a JSON array of `{id, text, payload, received_at_ms}` (without the placeholder the JSON is appended) |
| `result_ttl_days` | `7` | how long a turn's result is kept |

Sources are read at startup, like the rest of `[inbound]`. Each one gets its own policy at the head of the module's pipeline: acknowledged at ingest, weighted into the interactive lane, released at once, and debounced with its own windows (`sources[].coalesce` in `tinyeventqueues`, which needs a module release that has it). Events of one source and key coalesce into one batch; each batch runs as one turn of the pinned agent on a fresh thread, under a non-interactive origin whose external-effect tool calls park for approval (as a remote trigger's do), so the agent should be tool-less or read-only. Each event's text is screened on its own with the prompt-injection guard `inference_agent_chat` applies: a blocked event is left out of the prompt and listed on the result as `blocked: [{event_id, reason}]` (the reason is the guard's verdict and rule codes, never the text), so one injected event cannot fail or hold back its batch. A batch with every event blocked completes with empty `text`, without calling the model, and still publishes `InboundTurnCompleted`. The guard still checks the rendered prompt as a whole; if that fires the turn fails and is retried like any other failure. A turn that fails is retried with the lane's back-off and stored as failed after its last attempt; a turn still running at the delivery budget is acknowledged and stores its result when it ends; a turn left running when the core stopped is run again at the next start.

| RPC | Params | Result |
| --- | --- | --- |
| `openhuman.inbound_offer` | `source`, `key`, `event_id`, `text`, `payload?`, `received_at_ms?` | `{disposition}`: `accepted`, or `duplicate` for a redelivered `event_id` (a success); an unconfigured source is an error |
| `openhuman.inbound_results` | `source?`, `since_ms?`, `limit?` (50, at most 500), `status?` (`running`, `completed`, `failed`) | `{results: [{turn_id, source, key, agent_id, status, text?, error?, event_ids, blocked, created_ms, completed_ms?}]}`, oldest first, acknowledged results left out |
| `openhuman.inbound_result_ack` | `turn_id` | `{acked: true}`; an unknown turn is an error |

| Event (domain `inbound`) | Fields | When |
| --- | --- | --- |
| `InboundTurnCompleted` | `source`, `key`, `turn_id`, `event_ids`, `agent_id` | a batch's agent answered and its text is stored |
| `InboundTurnFailed` | `source`, `key`, `turn_id`, `detail` | a batch failed for good |

**The SSE stream wakes you; the RPC returns the result.** The events carry no agent text, and `/events/domain` only carries a short detail (`x:mentions/mentions turn <id>: 3 event(s)`). A consumer subscribes to the stream (or polls), calls `inbound_results` when woken, handles each result, and acknowledges it with `inbound_result_ack`.

The queues' gate still applies to an external source: its sender is the source itself, so more than `gate.rate_limits.per_sender` events a minute (30 in the shipped pipeline) spill into the digest, and near-duplicate texts lose salience to habituation. A digest turn of one pinned source's events is still run on its pinned agent.

## Wallet

`wallet = false` removes the `web3` domain family and tool group, so `wallet.*`, `web3.*` and `x402.*` answer `COMPONENT_DISABLED:wallet`. The x402 payment hook behind `http_request` declines before any quote or signing (the 402 response comes back unpaid), and `tinywallet` is never downloaded, attested or loaded. Keychain material, spending limits (`OPENHUMAN_X402_*_MAX`) and RPC endpoints (`OPENHUMAN_WALLET_RPC_*`) are left as they are, so setting `wallet = true` again restores the wallet on the next core start.

## Voice

`voice = false` removes the `voice` domain family, so `voice.*` answers `COMPONENT_DISABLED:voice`, and `tinyvoice` is never downloaded, attested or loaded. The former config controllers for dictation and voice-server settings have been retired; their legacy keys remain migration inputs only and no longer control voice startup. `voice_server.stt_engine` remains as the fallback engine when no explicit STT provider is selected. `about_app` and `modules.list` report voice as disabled. The Cargo `voice` feature is unchanged.

## Sandbox

`sandbox = false` means the TinyBox module is never downloaded, attested, loaded or initialised, and `sandbox.*` answers `COMPONENT_DISABLED:sandbox`. Sandboxed execution takes the documented fallback, the same as `[sandbox] enabled = false` or `OPENHUMAN_SANDBOX=off`: the command runs unjailed and the Rust path checks, the always-forbidden credential floor and the security policy still apply. TinyBox registers no tools and starts no background services. `modules.list` reports `tinybox` as `disabled`, `about_app` marks the sandbox capability as disabled, and the host `DomainSet` still wins. A SaaS core still requires the per-user container path, so with the module disabled it fails closed instead of running unsandboxed.
