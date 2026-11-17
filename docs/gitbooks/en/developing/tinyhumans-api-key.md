---
description: >-
  What one TinyHumans API key unlocks, how to use it and how OpenHuman stores
  and sends it.
icon: key
---

# One TinyHumans API key

One TinyHumans credential unlocks every managed backend service: chat inference over the OpenRouter model catalogue, web search, embeddings, media generation, integrations, voice and the Jev ranker. There is no separate key per service. The desktop app gets the credential when you sign in. A library host or a headless process supplies it directly.

## How to get one

All managed services sit behind one TinyHumans account. Sign in through the desktop app to get a session credential, or generate an API key (`th_...`) for library and headless use.

## Using it

Desktop app: sign in with a TinyHumans account. The app stores a session credential in the OS-backed auth-profile store. The core resolves it the same way it resolves an API key, through `credentials::api_key` and `session_state::resolve_backend_credential`.

Library: pass the key to `RuntimeBuilder` and it boots connected:

```rust
use openhuman_tinyhumans::{embed::Workspace, RuntimeBuilder};

let runtime = RuntimeBuilder::new()
    .workspace(Workspace::Ephemeral)
    .api_key("th_...")
    .build()
    .await?;
```

`openhuman_tinyhumans::RuntimeBuilder` mirrors `openhuman_embed::RuntimeBuilder` method for method. On `build()` it installs the SDK-backed transport and binds it to the runtime.

Headless: set `OPENHUMAN_BACKEND_API_KEY` before the core boots. The boot sequence (`crates/openhuman-core/src/credentials/ops/boot_env.rs`) installs it on a fresh credential store and never overwrites a credential that is already there.

## Where it is stored and how it is sent

The key lives in the same auth-profile store as an app session, under its own provider id (`api-key`). It is never written to `config.toml` and never logged. It is the only credential a library embedder holds. There is no login-token exchange, no session JWT and no `/auth/me` round trip.

The key is sent in one of two forms, depending on the endpoint:

- Managed inference (`{api_url}/openai/v1`): `Authorization: Bearer <key>`. `OpenHumanBackendModel::resolve_bearer` prefers the stored API key over an app session when both exist.
- Backend REST (`BackendClient`, `IntegrationClient`): `x-api-key: <key>`.

`crates/openhuman-core/src/credentials/api_key.rs` owns storage and retrieval (`store_api_key`, `get_api_key`, `has_api_key`, `clear_api_key`). `session_state::BackendCredential` turns either credential form into the right header at request time.

## Bring your own key still works

The API key covers the managed path. You can still point any workload at your own provider: chat and reasoning at a bring-your-own-key LLM slug, embeddings at Voyage, OpenAI or Cohere directly, and search at Brave, Exa or Tavily directly. See [Pluggable engines](engines.md) for the full list and the config key for each. Running everything with your own keys needs no TinyHumans key. Running anything through the managed backend does.
