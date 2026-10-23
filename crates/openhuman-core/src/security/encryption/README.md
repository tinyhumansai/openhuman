# Encryption

AES-256-GCM at-rest crypto for AI memory storage and the encrypt/decrypt RPC surface. Owns the encrypted-payload format, Argon2id password-derived keys, and the data-directory resolver. The `encrypt_secret` / `decrypt_secret` RPCs are thin shims that delegate to the credentials domain: this module is intentionally small and composable, not a key-management service.

## Public surface

- Removed: the host `EncryptedPayload`/`EncryptionKey` primitives (`core.rs`) now live in the tinysecurity module (`PasswordCrypt2`, `modules/security_crypto.rs`); the legacy payload layout is unchanged and pinned by `modules/fixtures/legacy_password_payload.json`.
- `pub async fn encrypt_secret(config: &Config, plaintext: &str) -> Result<Outcome<String>, String>` (`ops.rs:6`): RPC handler, delegates to `credentials::rpc::encrypt_secret`.
- `pub async fn decrypt_secret(config: &Config, ciphertext: &str) -> Result<Outcome<String>, String>` (`ops.rs:13`): RPC handler, delegates to `credentials::rpc::decrypt_secret`.
- RPC `encryption.{encrypt_secret, decrypt_secret}`, defined in [`schemas.rs`](./schemas.rs) (re-exported via `all_encryption_controller_schemas` / `all_encryption_registered_controllers`).
- Constants: `SALT_LENGTH = 16`, `NONCE_LENGTH = 12`, `KEY_LENGTH = 32` (private but stable parameters).

## Calls into

- `argon2` crate for `Argon2id` password-derived keys.
- `aes-gcm` crate for `Aes256Gcm` AEAD.
- `crates/openhuman-core/src/config/`: `Config` for workspace-relative data directory.
- `crates/openhuman-core/src/security/credentials/`: `credentials::rpc::{encrypt_secret, decrypt_secret}` carry the actual key-management responsibility.

## Called by

- `crates/openhuman-core/src/security/credentials/`: uses the same `EncryptedPayload` / `EncryptionKey` primitives directly when storing per-channel secrets.
- `crates/openhuman-core/src/core/all.rs`: registers `all_encryption_*` controllers so the shell and CLI can encrypt configuration secrets.
- Indirect: `crates/openhuman-core/src/memory/`, `crates/openhuman-core/src/channels/`, and `crates/openhuman-core/src/inference/local/` rely on the credentials domain (which in turn uses this layer) for secrets at rest.

## Tests

- This domain has no `*_tests.rs` siblings; the underlying crypto round-trips are exercised by `crates/openhuman-core/src/security/secret_storage/encrypted_store_tests.rs` and the credentials tests, which both cover encrypt/decrypt happy paths and tampered-ciphertext rejection.

## Further reading

- [Parent module (`security`)](../README.md)
- [Security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md)
- [Privacy and security](../../../../../docs/gitbooks/en/features/privacy-and-security.md)
- [Approval gate](../../../../../docs/gitbooks/en/features/approval-gate.md)
- [OS keyring and secret storage](../../../../../docs/gitbooks/en/features/os-keyring-and-secret-storage.md)
