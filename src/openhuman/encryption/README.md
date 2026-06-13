# Encryption

AES-256-GCM at-rest crypto for AI memory storage and the encrypt/decrypt RPC surface. Owns the encrypted-payload format, Argon2id password-derived keys, and the data-directory resolver. The `encrypt_secret` / `decrypt_secret` RPCs are thin shims that delegate to the credentials domain — this module is intentionally small and composable, not a key-management service.

## Public surface

- `pub struct EncryptedPayload` — `core.rs:18-26` — `{ ciphertext, nonce, salt }` triple persisted to disk.
- `pub struct EncryptionKey` — `core.rs:29-32` — `[u8; 32]` AES-256 key wrapper.
- `impl EncryptionKey::derive(password: &str, salt: &[u8]) -> Result<Self, String>` — `core.rs:35` — Argon2id with parameters `m=65536, t=3, p=1`.
- `pub fn get_data_dir() -> Result<PathBuf, String>` — `core.rs` — resolve the encrypted-data directory under the openhuman workspace.
- `pub async fn encrypt_secret(config: &Config, plaintext: &str) -> Result<RpcOutcome<String>, String>` — `ops.rs:6` — RPC handler, delegates to `credentials::rpc::encrypt_secret`.
- `pub async fn decrypt_secret(config: &Config, ciphertext: &str) -> Result<RpcOutcome<String>, String>` — `ops.rs:13` — RPC handler, delegates to `credentials::rpc::decrypt_secret`.
- RPC `encryption.{encrypt_secret, decrypt_secret}` — `schemas.rs` (re-exported via `all_encryption_controller_schemas` / `all_encryption_registered_controllers`).
- Constants: `SALT_LENGTH = 16`, `NONCE_LENGTH = 12`, `KEY_LENGTH = 32` (private but stable parameters).

## Calls into

- `argon2` crate for `Argon2id` password-derived keys.
- `aes-gcm` crate for `Aes256Gcm` AEAD.
- `src/openhuman/config/` — `Config` for workspace-relative data directory.
- `src/openhuman/credentials/` — `credentials::rpc::{encrypt_secret, decrypt_secret}` carry the actual key-management responsibility.

## Called by

- `src/openhuman/credentials/` — uses the same `EncryptedPayload` / `EncryptionKey` primitives directly when storing per-channel secrets.
- `src/core/all.rs` — registers `all_encryption_*` controllers so the shell + CLI can encrypt configuration secrets.
- Indirect: `src/openhuman/memory/`, `src/openhuman/channels/`, and `src/openhuman/inference/local/` rely on the credentials domain (which in turn uses this layer) for secrets at rest.

## Tests

- This domain has no `*_tests.rs` siblings; the underlying crypto round-trips are exercised by `src/openhuman/keyring/encrypted_store_tests.rs` and the credentials tests, which both cover encrypt/decrypt happy paths and tampered-ciphertext rejection.
