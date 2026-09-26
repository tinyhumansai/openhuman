# webhooks (hosted)

Backend-managed webhook tunnels, on the TinyHumans SDK's typed `webhooks()`
client (`/webhooks/core*`). A thin, stateless RPC adapter: the backend owns
tunnel provisioning, ownership and bandwidth quotas.

| Method | Inputs | Backend call |
| --- | --- | --- |
| `webhooks_list_tunnels` | none | `GET /webhooks/core` |
| `webhooks_create_tunnel` | `name`, `description?` | `POST /webhooks/core` |
| `webhooks_get_tunnel` | `id` | `GET /webhooks/core/{id}` |
| `webhooks_update_tunnel` | `id`, `name?`, `description?`, `isActive?` | `PATCH /webhooks/core/{id}` |
| `webhooks_delete_tunnel` | `id` | `DELETE /webhooks/core/{id}` |
| `webhooks_get_bandwidth` | none | `GET /webhooks/core/bandwidth` |

The `webhooks` namespace is shared with the core: the local webhook router
(`crates/openhuman-core/src/skills/webhooks/` — `list_registrations`,
`register_echo`, `register_agent`, `trigger_agent`, the debug log ring) stays
in the core because it routes inbound deliveries to skills and agents and
needs no TinyHumans account. These six arrive with
`openhuman_tinyhumans::install` and are absent from a core without it.

Credentials, error mapping and the no-account short-circuit come from
`hosted::client::HostedClient` (see `../client.rs`).
