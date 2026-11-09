# Secret encryption RPC compatibility surface

This module preserves the `encryption.encrypt_secret` and
`encryption.decrypt_secret` RPC names. The handlers delegate to the credentials
domain, which uses TinySecurity's cipher bus and the host's consent and key
storage adapters. This module owns no cipher implementation or key material.

## Public surface

- `pub async fn encrypt_secret(config: &Config, plaintext: &str) -> Result<Outcome<String>, String>` (`ops.rs:6`): RPC handler, delegates to `credentials::ops::encrypt_secret`.
- `pub async fn decrypt_secret(config: &Config, ciphertext: &str) -> Result<Outcome<String>, String>` (`ops.rs:13`): RPC handler, delegates to `credentials::ops::decrypt_secret`.
- RPC `encryption.{encrypt_secret, decrypt_secret}`, defined in [`schemas.rs`](./schemas.rs) (re-exported via `all_encryption_controller_schemas` / `all_encryption_registered_controllers`).
- Constants: `SALT_LENGTH = 16`, `NONCE_LENGTH = 12`, `KEY_LENGTH = 32` (private but stable parameters).

## Calls into

- `crates/openhuman-core/src/credentials/`: `credentials::ops::{encrypt_secret, decrypt_secret}` route operations through TinySecurity.
- `crates/openhuman-core/src/modules/security/`: host-side bus client and callback binding.

## Called by

- `crates/openhuman-core/src/core/all.rs`: registers `all_encryption_*` controllers so the shell and CLI can encrypt configuration secrets.

## Tests

The owning credentials and SecretStore test modules cover the host's TinySecurity adapter behavior.

## Further reading

- [Parent module (`security`)](../README.md)
- [Security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md)
- [Privacy and security](../../../../../docs/gitbooks/en/features/privacy-and-security.md)
- [Approval gate](../../../../../docs/gitbooks/en/features/approval-gate.md)
- [OS keyring and secret storage](../../../../../docs/gitbooks/en/features/os-keyring-and-secret-storage.md)
