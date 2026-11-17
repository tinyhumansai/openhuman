---
description: "Pin connector credentials per agent while the host supplies backend transport."
icon: code
---

# Composio

Composio calls use the host backend transport. Install that transport before booting a managed runtime; see [TinyHumans managed inference](tinyhumans-managed.md). An explicit inference provider route supplies model inference, not the connector transport.

Use `AgentSpec::composio(ComposioHostCredential)` to pin a connector credential to an agent. This keeps the credential selection on the host side instead of asking the model to choose another user's credential. Configure the agent's tool scope and access separately. The connector tools still pass through the host's execution policy.

The repository's [Composio agent integration test](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/tests/composio_agents.rs) verifies two agents using their own pinned credentials against a loopback backend. It checks the outgoing authentication and action results rather than contacting Composio. Run the Embed integration suite through the repository's cancellation-aware runner when changing this adapter.

Account login, linking, and credential acquisition belong to your connected host and connector layer. Keep secrets out of prompts and capability reports. For multiple users, use [SaaS profiles](../guides/saas-multi-tenant.md); independent action directories on a shared library runtime do not establish tenant isolation.
