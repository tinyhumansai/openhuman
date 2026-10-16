# integrations

Agent integrations that run through the core itself: Composio connectors
([`composio/`](composio/README.md)) and task sources
([`task_sources/`](task_sources/README.md)).

The hosted `/agent-integrations/*` client (`IntegrationClient`), its managed
tools (Google Places, stock data, file storage, managed media generation), the
web3 crypto client and the Composio backend route are **not** here. They live in
`crates/openhuman-tinyhumans/src/integrations/` and plug in through generic
registries:

- `tools::sources` (tool sources) appends their tools to the catalogue.
- `integrations::composio::client::register_composio_route_builder` and
  `modules::connectors::register_proxy_target` serve `composio.mode = backend`.
  With neither registered the mode answers `BACKEND_UNAVAILABLE:` and the
  composio agent tools do not register; direct mode is unaffected.
- `modules::wallet_backend::register_wallet_backend` answers the wallet
  module's backend callback; unregistered it answers `BACKEND_UNAVAILABLE:`.
- `media::generation::media_tools_for_config` builds the `media_generate_*`
  tools over generators a host supplies.

## RPC surface

This folder registers no controllers of its own. Its children do, both under
`DomainGroup::Integrations` in `core/all.rs`: `composio.*`
(see [composio/README.md](composio/README.md)) and `task_sources.*`
(see [task_sources/README.md](task_sources/README.md)).

## Gotchas

- Composio tests that exercise backend mode register the stand-in route in
  `composio/test_backend_route.rs` (thread-local in `cfg(test)`); the core ships
  no backend route.
- Web search is not here: provider implementations live in the
  `vendor/tinysearch` module, host policy in [`search/`](../search/).
