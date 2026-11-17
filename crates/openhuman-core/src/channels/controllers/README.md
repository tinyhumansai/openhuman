# Controllers

Owns the channels RPC namespace: provider descriptions, connection settings,
status, messaging and hosted authentication flows. Provider execution uses the
TinyChannels module through its minimal bus contract.

| Path | Purpose |
| --- | --- |
| [backend.rs](./backend.rs) | OpenHuman product callbacks for persisted configuration, reactions and threads, served through the registered channel callback (`callback_slot`); sends prepared messages through the confidential module member. |
| [definitions.rs](./definitions.rs) | Provider identifiers, capabilities and field declarations from tinychannels-bus. |
| [schemas.rs](./schemas.rs) | Existing RPC names, contract schemas, parameter decoding and thin host handlers. |
| ops/ | Credential custody, configuration persistence, disconnect cleanup and product adapters. |

Provider descriptions are contract vocabulary and need no running module.
Connection preparation and testing use confidential ChannelController requests;
the module validates provider credentials and prepares configuration updates.
OpenHuman persists the requested provider, keeps encrypted credentials, and
excludes email passwords and Yuanbao application secrets from TOML.

Message handlers ask the module to prepare outbound intent and execute
SendChannelIntent. Unavailable modules fail the affected operation. Hosted
reactions, thread management and Discord metadata remain narrow product API
callbacks. Managed Telegram and Discord account-link methods are registered by
openhuman-tinyhumans through its hosted channel-link extension.

The core registry mounts these controllers under DomainGroup::Channels and the
channels feature. Hosted inbound relay registration is included; in-app web chat
has its own channel namespace.

Host tests cover RPC outcomes, credentials and persistence using contract
fixtures. Provider validation, normalization, transport and delivery regressions
belong to TinyChannels.

- [Backend tests](./backend_tests.rs)
- [Operation tests](./ops_tests.rs)
- [Schema tests](./schemas_tests.rs)
- [Channels overview](../README.md)
- [Messaging channels](../../../../../docs/gitbooks/en/features/channels.md)
