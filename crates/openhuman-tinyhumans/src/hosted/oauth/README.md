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
| `auth_oauth_fetch_client_key` | `integrationId` (24-char hex) | `POST /auth/integrations/{id}/client-key` (session only; raw SDK request, no typed method) |

These share the `auth` namespace with the core's credential controllers
(`auth.set_credential`, `auth.get_state`, provider credentials, …), which stay
in the core. `auth_oauth_fetch_client_key`'s route has no typed SDK method and
is absent from the backend's published contract, so it is sent through the
SDK's raw primitive, which still applies the SDK's route policy.

The core's `oauth_connect_url` / `oauth_list` agent tools reach these through
the controller registry, so they report `BACKEND_UNAVAILABLE:` on a core
without `openhuman_tinyhumans::install`.

## Further reading

- [`docs/gitbooks/en/developing/tinyhumans-api-key.md`](../../../../../docs/gitbooks/en/developing/tinyhumans-api-key.md): running on a TinyHumans API key.
- [`crates/openhuman-tinyhumans/src/hosted/README.md`](../README.md): hosted proxies.
- [`vendor/tinyhumans-sdk/README.md`](../../../../../vendor/tinyhumans-sdk/README.md): tinyhumans-sdk.
