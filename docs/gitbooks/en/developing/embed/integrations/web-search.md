---
description: "Pin a web-search provider key per agent, held only in memory."
icon: code
---

# Web search

An agent's web-search tools (`web_search_tool`, `web_answer_tool`, `web_contents_tool`) run through the process-wide TinySearch module. Several agents in one runtime can each search with their own provider key: pin it on that agent's config, and it is never written to `config.toml`, the environment, or a log.

In the agent's `AgentSpec::config` closure, call `config.search.pin_host_credential("brave", SearchHostCredential::new(key))`. `SearchHostCredential` is exported from `openhuman_embed`.

`pin_host_credential` accepts `brave`, `exa`, `gemini`, `parallel`, `querit`, `tavily` and `tinyfish`, and refuses any other provider with `UnknownKeyedSearchProvider`. It turns search on and puts that provider on its direct route, the route a key authenticates; a pinned `gemini` key also serves deep research. `SearchHostCredential::with_base_url` sends the provider's calls to another endpoint, such as a proxy or a local stand-in under test.

While an agent's config carries a pinned key, its search calls run under that config instead of a reload of the runtime's `config.toml`. The module is reconfigured whenever the next call needs a different configuration, and calls take turns, so one agent's call never runs with another agent's key. Agents without a pinned key keep the runtime's settings, environment and default managed routing.

Provider order per role still comes from `search.roles` and TinySearch's defaults. A runtime signed in to the managed backend serves the managed providers first; set `search.roles` on the agent to put the pinned provider first.

The repository's [pinned-key integration test](https://github.com/tinyhumansai/openhuman/blob/main/tests/search_pinned_keys_e2e.rs) runs two agents with different keys against a loopback provider. It checks each call's outgoing key, and that no key reaches the environment, a file or a log line.
