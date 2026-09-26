# oauth (hosted)

Backend-brokered OAuth integrations on the TinyHumans SDK's typed `auth()`
client. Stateless RPC adapter; the backend runs the OAuth dance and stores
the provider tokens.

| Method | Inputs | Backend call |
| --- | --- | --- |
| `auth_oauth_connect` | `provider`, `skillId?`, `responseType?`, `encryptionMode?` | `GET /auth/{provider}/connect` |
| `auth_oauth_list_integrations` | none | `GET /auth/integrations` |
| `auth_oauth_fetch_integration_tokens` | `integrationId`, `key` | `POST /auth/integrations/{id}/tokens` (AES-GCM handoff, decrypted with the core's `api::decrypt_handoff_blob`) |
| `auth_oauth_revoke_integration` | `integrationId` | `DELETE /auth/integrations/{id}` |

These share the `auth` namespace with the core's credential controllers
(`auth.set_credential`, `auth.get_state`, provider credentials, …), which stay
in the core. `auth_oauth_fetch_client_key` also stays in the core: its route
(`POST /auth/integrations/{id}/client-key`) has no SDK method and is missing
from the SDK's public-route registry.

The core's `oauth_connect_url` / `oauth_list` agent tools reach these through
the controller registry, so they report `BACKEND_UNAVAILABLE:` on a core
without `openhuman_tinyhumans::install`.
