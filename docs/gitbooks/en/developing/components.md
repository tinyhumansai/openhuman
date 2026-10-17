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
memes = true       # tinymemes
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
| `OPENHUMAN_TINYMEMES` set to an off value | `components.memes = false` |
| `OPENHUMAN_DISABLE_CHANNEL_LISTENERS=1` | `components.channels = false` at load (env override, kept for one release; `OPENHUMAN_COMPONENT_CHANNELS` wins) |
| `[dictation] enabled`, `[voice_server] auto_start` and `always_on_enabled` all explicitly `false` | `components.voice = false` (any other mix keeps voice on) |
| `[sandbox] enabled = false` (or `[security.sandbox] enabled = false`) | `components.sandbox = false` (`enabled = true` or unset keeps it on; the old key is kept) |
| `[composio] enabled` | dropped; it was never read |

A `components.*` key already present in the file always wins over its legacy flag, so the migration is idempotent and turning a component back on sticks.

## Memes

`components.memes` is the master switch: off means no remix and no module load, and the A/B arm is ignored. When on, `OPENHUMAN_TINYMEMES` (`off`, `on`, `ab`, `ab:NN`; build-time default `OPENHUMAN_TINYMEMES_DEFAULT`) picks the threads that are remixed. The `[memes]` section tunes it: `timeout_ms` (default 20000) and `meme_cooldown` (default 3). `OPENHUMAN_TINYMEMES_TIMEOUT_MS` and `OPENHUMAN_TINYMEMES_MEME_COOLDOWN` override them when set.

The module reads its own `TINYMEMES_*` overrides from the process environment, so credentials never cross the bus: `TINYMEMES_OPENROUTER_KEY` (and `TINYMEMES_MODEL`) route the rewrite model and slang research through OpenRouter, `TINYMEMES_JEV` (`auto`, `typesafe`, `openrouter`, `openjev`, `llm`) with `TINYMEMES_TYPESAFE_KEY` / `TINYMEMES_OPENJEV_KEY` picks the Jev route. Without any override the module asks OpenHuman for Jev; when OpenHuman has none (signed out, offline session) the chat model answers Jev's questions instead. After the reply is delivered the module may run slang and meme research in the background through OpenHuman web search. Learned slang, memes and the remixed-message ring are documents on the storage port (older `<workspace>/tinymemes/*.json` files are adopted once and renamed `*.migrated`).

## Wallet

`wallet = false` removes the `web3` domain family and tool group, so `wallet.*`, `web3.*` and `x402.*` answer `COMPONENT_DISABLED:wallet`. The x402 payment hook behind `http_request` declines before any quote or signing (the 402 response comes back unpaid), and `tinywallet` is never downloaded, attested or loaded. Keychain material, spending limits (`OPENHUMAN_X402_*_MAX`) and RPC endpoints (`OPENHUMAN_WALLET_RPC_*`) are left as they are, so setting `wallet = true` again restores the wallet on the next core start.

## Voice

`voice = false` removes the `voice` domain family, so `voice.*` answers `COMPONENT_DISABLED:voice`. The embedded voice server, the dictation hotkey listener and always-on listening do not start whatever `[dictation] enabled`, `[voice_server] auto_start` and `always_on_enabled` say, and `tinyvoice` is never downloaded, attested or loaded. `about_app` and `modules.list` report voice as disabled. The Cargo `voice` feature is unchanged.

## Sandbox

`sandbox = false` means the TinyBox module is never downloaded, attested, loaded or initialised, and `sandbox.*` answers `COMPONENT_DISABLED:sandbox`. Sandboxed execution takes the documented fallback, the same as `[sandbox] enabled = false` or `OPENHUMAN_SANDBOX=off`: the command runs unjailed and the Rust path checks, the always-forbidden credential floor and the security policy still apply. TinyBox registers no tools and starts no background services. `modules.list` reports `tinybox` as `disabled`, `about_app` marks the sandbox capability as disabled, and the host `DomainSet` still wins. A SaaS core still requires the per-user container path, so with the module disabled it fails closed instead of running unsandboxed.
