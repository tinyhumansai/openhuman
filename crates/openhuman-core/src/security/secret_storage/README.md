# Secret storage

Host-installable secret storage with an in-memory default, plus a ChaCha20-Poly1305 encrypted secret store for config-field encryption. Native keychains live in `openhuman-rpc::keyring`; this module has no OS credential-store dependency. This is an infrastructure domain: it has no RPC controllers, no agent tools, and no event-bus subscribers. It provides a namespaced, user-scoped `get`/`set`/`delete` interface over a backend selected from a host install slot or cached per process, and a separate `SecretStore` type that other domains use to encrypt and decrypt secret values stored in config. All keys are scoped under a `user_id` so multiple users coexist without collision; the backend entry key format is `"{user_id}:{logical_key}"`.

## Responsibilities

- Provide `get` / `set` / `delete` / `get_or_create_random` over a process-global secret-storage backend.
- Resolve the host-installed backend first, then an explicit environment override, then the in-memory default. Cache the fallback in a `OnceLock`.
- Probe and cache backend availability (`is_available`) so callers (wallet guards, snapshot loops) can fall back to file storage without re-triggering OS keychain prompts.
- Migrate a secret from a plaintext file into a persistent backend (`migrate_from_file`), verifying the write before deleting the source. Volatile providers reject migration and retain the source.
- Encrypt and decrypt config-field secrets via `SecretStore` (ChaCha20-Poly1305, `enc2:` prefix), including migration of the legacy XOR `enc:` format.
- Load and cache a single app-scoped master encryption key at startup (`init_master_key`) for the `encrypted_file` backend: from `OPENHUMAN_KEYRING_MASTER_KEY` / `OPENHUMAN_KEYRING_MASTER_KEY_FILE` when an operator supplies one (headless deployments with no keychain, #6926), otherwise from a host-installed `MasterKeySource`, then a private workspace key file.
- Migrate the legacy plaintext `dev-keychain.json` into the encrypted `secrets.enc` file on first use.

`MemoryBackend` is a thread-safe map with no disk or OS access. Each instance owns its values; the default instance is cached process-wide. Install a custom `SecretBackend` before boot when an embedder needs persistence.

## Key files

| File | Role |
| --- | --- |
| `crates/openhuman-core/src/security/secret_storage/mod.rs` | Module docstring and exports only. Documents backend-selection priority and the Linux-headless availability note. Re-exports the public surface. |
| `crates/openhuman-core/src/security/secret_storage/ops.rs` | Core operations: `get`/`set`/`delete`/`is_available`/`get_or_create_random`/`migrate_from_file`, the `MigrationOutcome` enum, the `namespaced_key` helper, and the cached availability probe. |
| `crates/openhuman-core/src/security/secret_storage/store.rs` | Backend selection and global state. Owns the `WORKSPACE_DIR` and `BACKEND` `OnceLock`s, `init_workspace`, `backend()`, `build_backend()`/`build_backend_at()`, and `workspace_dir_for_file_backend()` path derivation. Also the `cfg(test)`-only `test_scope` module (thread-scoped test workspaces). |
| `crates/openhuman-core/src/security/secret_storage/memory_backend.rs` | Basic in-memory backend, default for core and embed; no key initialization or secret files. |
| `crates/openhuman-core/src/security/secret_storage/backend.rs` | `SecretBackend` trait plus `FileBackend` (plaintext `dev-keychain.json`, test/debug only), and test-only `MockBackend`. |
| `crates/openhuman-core/src/security/secret_storage/encrypted_file_backend.rs` | `EncryptedFileBackend`: an adapter over `tinystoragedrivers`' `EncryptedFileSecrets` (format and `secrets.enc.lock` unchanged): all secrets in one ChaCha20-Poly1305 `secrets.enc` file keyed by an app master key; `init_master_key`; legacy `dev-keychain.json` migration; corrupt-file quarantine (a desktop policy kept here: the driver fails closed, this adapter moves the file aside as `secrets.enc.corrupt.<ts>` and continues, so a bad file never wedges sign-in). `adapter.rs` holds the shared error mapping and that policy; `fixtures/` holds bytes written by the pre-port code. |
| `crates/openhuman-core/src/security/secret_storage/encrypted_store.rs` | `SecretStore`: config-field encryption (`enc2:` ChaCha20-Poly1305, legacy `enc:` XOR migration), keychain-backed master key with legacy `.secret_key` file migration, process-wide key cache, Windows ACL repair (`icacls`). |
| `crates/openhuman-core/src/security/secret_storage/file_store.rs` | Shared secrets-file primitives for both file backends: the cross-process advisory write lock (`lock_for_write`, on a sidecar `<path>.lock`), the `0600` unique-temp-then-rename `write_atomic`, and `quarantine_corrupt`. |
| `crates/openhuman-core/src/security/secret_storage/crypto.rs` | Shared ChaCha20-Poly1305 helpers (`chacha20_encrypt`/`chacha20_decrypt`), random-byte generation, hex encode/decode. Used by both `encrypted_store` and `encrypted_file_backend`. |
| `crates/openhuman-core/src/security/secret_storage/error.rs` | `SecretStorageError` (thiserror) with variants `InvalidUtf8`/`MigrationReadFailed`/`VerifyFailed`/`MigrationDeleteFailed`/`RandomGeneration`/`Crypto`/`Backend`, plus a log-safe `diagnostic()`; native errors arrive as opaque `Backend` detail. |
| `crates/openhuman-core/src/security/secret_storage/mod_tests.rs` | Module tests. |
| `crates/openhuman-core/src/security/secret_storage/store_tests.rs`, [`store_tests_2_tests.rs`](./store_tests_2_tests.rs), [`store_test_scope_tests.rs`](./store_test_scope_tests.rs) | Test-isolation regressions: test builds ignore `OPENHUMAN_WORKSPACE`, production resolution still honours it, scoped workspaces do not share secrets, and a deleted scoped workspace cannot reset the default store. |
| `crates/openhuman-core/src/security/secret_storage/encrypted_store_tests.rs`, [`encrypted_store_crypto_migration_tests.rs`](./encrypted_store_crypto_migration_tests.rs), [`encrypted_store_key_management_tests.rs`](./encrypted_store_key_management_tests.rs) | `SecretStore` tests (wired via `#[path]` from [`encrypted_store.rs`](./encrypted_store.rs)). |

## Public surface

Re-exported from [`mod.rs`](./mod.rs):

- `SecretBackend`: backend trait (`get`/`set`/`delete`/`name`).
- `install_secret_backend`, `installed_secret_backend`, `clear_secret_backend`: host install slot; thread-local in unit tests.
- `MasterKeySource` (`load`/`store`) and `install_master_key_source`, `installed_master_key_source`, `clear_master_key_source`: host key-source slot, with the same isolation.
- `SecretStore`: config-field encrypt/decrypt; `encrypt`/`decrypt`/`decrypt_and_migrate`/`needs_migration`/`is_encrypted`/`new`.
- `SecretStorageError`: error enum with `diagnostic()`.
- `init_master_key`: load the app master key at startup for encrypted storage, from the environment (`OPENHUMAN_KEYRING_MASTER_KEY` inline, or `OPENHUMAN_KEYRING_MASTER_KEY_FILE` naming a file; 64 hex characters, exactly one of the two) when set, otherwise from the host source or `keyring/master.key`. A set-but-malformed variable is a boot error, not a fall-through; the source is logged at `info`, the value never. Key files must not be group- or world-writable; read-only group/world access is supported for secret mounts, but any principal able to read the file can decrypt the keyring.
- `init_workspace`: register the workspace dir for file and encrypted-file backends.
- `get`, `set`, `delete`, `get_or_create_random`, `is_available`, `migrate_from_file`, `MigrationOutcome`.

## RPC / controllers

None. The secret-storage domain exposes no `schemas.rs`, no `all_*_controller_schemas`, and no `openhuman.keyring_*` methods. It is consumed in-process by other domains.

## Agent tools

None.

## Events

None. There is no `bus.rs`, and the module publishes and subscribes to no `DomainEvent`.

## Persistence

Secret storage backend: host install slot first, then a cached environment override or default. Native implementations live in `crates/openhuman-rpc/src/keyring/`, behind its `os-keyring` feature.

- `os` (via `OPENHUMAN_KEYRING_BACKEND=os`): host-installed native OS credential store, macOS Keychain, Windows Credential Manager, or Linux Secret Service, under service name `"openhuman"`.
- `memory` (core/embed default): unencrypted values in a thread-safe in-memory map, lost when the process exits.
- `encrypted_file` (explicit selection or RPC product-host default): single ChaCha20-Poly1305 file `{workspace}/secrets.enc`, encrypted with a master key loaded once, from `OPENHUMAN_KEYRING_MASTER_KEY` / `OPENHUMAN_KEYRING_MASTER_KEY_FILE` when set, otherwise from a host `MasterKeySource`, then `{workspace}/keyring/master.key`. Files are written `0600` on Unix via temp-file plus atomic rename.
- `file` (explicit `OPENHUMAN_KEYRING_BACKEND=file`): plaintext JSON `{workspace}/dev-keychain.json`. Not encrypted; test and debug use only.
- `mock` (test-only): in-memory `HashMap`.

`SecretStore` additionally manages a master encryption key: on the active process backend (slot `secretstore.master_key`) in normal builds with one-time migration into persistent providers from the legacy `{data_dir}/openhuman/.secret_key` file; memory providers read existing legacy keys without deleting them and keep new keys only in memory. Unit tests also retain the legacy file path. Decoded keys are cached process-wide, keyed by normalized path.

The workspace dir resolves from `init_workspace`, else `OPENHUMAN_WORKSPACE`, else `~/.openhuman` (or `~/.openhuman-staging` under `OPENHUMAN_APP_ENV=staging`). In `cfg(test)` builds only, that rule is bypassed; see the test-isolation note below.

Both file backends keep every secret in one file, so a `set` of one key rewrites all of them. That read-modify-write cycle is guarded by `file_store::lock_for_write`. An in-process mutex is not sufficient, because a desktop core, another process embedding the same core, and a `cargo test` run that inherited `OPENHUMAN_WORKSPACE` can all address the same path.

### On a storage backend

When the host configured a storage backend (`OPENHUMAN_STORAGE_URL` /
`[storage] url`, see `crate::storage`), `get`, `set` and `delete` serve the
secret from `storage::secrets` instead: one encrypted document per
namespaced key in the acting agent's storage scope, under a per-scope key
derived from the master key above. `is_available` reports `true` without
probing. The app-level keys stay on the process backend: the availability
probe, `get_or_create_random` and `migrate_from_file` (the config
encryption key, `secretstore.master_key`) use `process_get` / `process_set` /
`process_delete`. `backend_name` keeps naming the process backend, which is
what the consent UI describes. The `enc2:` cipher itself is
`tinystoragedrivers::secrets::crypto` (byte-compatible; its fixtures were
written by this module), used by `crypto.rs` and `SecretStore`.

## Dependencies

Internal modules: configuration and workspace resolution, storage-driver execution, and the core-owned secret-access consent policy. Native keychain providers are installed by RPC and accessed only through `SecretBackend` and `MasterKeySource`. File persistence uses `tinystoragedrivers::secrets`; encryption uses `chacha20poly1305`. This module has no `keyring` dependency.

## Used by

Discovered consumers (`crate::security::secret_storage::*`):

- `crates/openhuman-core/src/lib.rs` and `crates/openhuman-core/src/core/runtime/context.rs`: call `init_master_key()` at startup.
- `crates/openhuman-core/src/security/secrets.rs`, `crates/openhuman-core/src/security/mod.rs`: secret handling.
- `crates/openhuman-core/src/config/schema/load/secrets.rs`: `SecretStore::new` / `is_encrypted` to encrypt and decrypt config fields on load.
- `crates/openhuman-core/src/security/credentials/profiles.rs`, `credentials/ops.rs`: `SecretStore`, `is_available`, `get`/`set`/`delete` for per-profile credential storage.
- `crates/openhuman-core/src/security/keyring_consent/`: gates the OS-keyring-to-local fallback behind user consent and reports `backend_name()`/`is_available()` as a `KeyringStatus`; see [`../keyring_consent/README.md`](../keyring_consent/README.md).
- `crates/openhuman-core/src/web3/wallet/ops/state.rs`: `is_available`/`get`/`set` for the wallet mnemonic.
- `crates/openhuman-core/src/security/devices/rpc.rs`: device secret handling.

## Notes and gotchas

- Installed providers take precedence, then `OPENHUMAN_KEYRING_BACKEND` (`memory`/`os`/`file`/`encrypted_file`), then `MemoryBackend`. The fallback is cached for the process lifetime. Memory secrets do not survive a restart. RPC product hosts explicitly install a persistent default; embedders can install their own backend before runtime boot.
- Test builds resolve the workspace per thread, not per process. `WORKSPACE_DIR` and `OPENHUMAN_WORKSPACE` are shared by every concurrently running test, so consulting them made the whole test binary share one credential store pinned to whichever workspace won the race. When the winner was a `TempDir` held by a test's env guard, that directory was deleted at the end of that test, and because `FileBackend::read_map` treats a missing file as an empty map, the next write silently reset the store, so unrelated tests read their own freshly-written secrets back as `None`. Under `cfg(test)`, `workspace_dir_for_file_backend()` therefore ignores both and uses `test_scope::current_workspace()`: a thread-local override when a test binds one with `test_scope::ScopedWorkspace`, otherwise a stable per-process directory under the system temp dir. Backends are then cached per resolved directory. Two consequences worth knowing: test runs never read or write the developer's real `~/.openhuman/dev-keychain.json`, and a test that wants a private credential store must use `ScopedWorkspace`, since setting `OPENHUMAN_WORKSPACE` no longer steers the keyring. Library and standalone core defaults are in-memory; RPC product hosts explicitly select persistent storage.
- Cleaning up historical leakage: `node scripts/prune-dev-keychain.mjs` reports (and with `--apply` removes, after taking a backup) `dev-keychain.json` entries keyed by dead `TempDir` basenames, left behind before the isolation fix. Dry run by default.
- `is_available()` is cached after the first probe. The probe performs delete/set/get/delete round-trips on the `os` backend; running it per-call triggered repeated macOS permission dialogs and starved frequent pollers. Memory, plaintext and mock backends short-circuit to `true`; encrypted storage requires an available master key. A failed probe is logged at `warn` because it silently flips `use_keychain` off.
- `is_available()` answers "is the active backend usable", never "are secrets in the OS keychain". Because the file backends short-circuit to `true`, availability says nothing about where secrets are, which is why `keyring_consent::policy::active_mode_for` derives `active_mode` from `backend_name()` and consults availability only for the `os` backend. Deriving it from `is_available()` alone was issue #6076: the shipped staging/production app told users their secrets were in the OS keychain while they were in `{workspace}/secrets.enc`. Consumers that genuinely want the usability answer (wallet guards, snapshot loops) read `available` and are unaffected.
- `migrate_from_file` never deletes the source unless the verified write succeeds, so failures are retryable.
- `encrypted_file` corrupt or undecryptable files are quarantined (renamed `secrets.enc.corrupt.<ts>`) and treated as empty rather than crashing.
- A corrupt file is never overwritten. `file` reads degrade to an empty map (so a missing token means "sign in again"), but a `set`/`delete` over an unparseable file quarantines it and returns an error. Returning empty on the write path is what turned a corrupt file into a wipe: the write that followed persisted a map holding nothing but the key being set.
- Mutations hold a cross-process lock, on `<secrets file>.lock` rather than the secrets file itself, because `write_atomic` replaces the file by rename, so a lock on the old inode would guard nothing. Callers must hold it across the read and the write.
- The `SecretStore` master-key file is write-once. On Windows it survives transient AV-scanner sharing violations via retry/backoff and attempts `icacls` ACL self-repair on permission errors. Decoded keys are cached so repeated decrypts (for example, snapshot polls) hit memory.
- Legacy formats: `SecretStore` migrates `enc:` (XOR) to `enc2:` (ChaCha20-Poly1305) on decrypt. `EncryptedFileBackend` writes `dev-keychain.json` into `secrets.enc`, reads back and verifies the encrypted map, then removes the plaintext source. It also removes older `.json.migrated` copies when their entries match the decrypted store. Invalid or divergent copies remain for manual recovery.
- Errors never carry secret values, only namespaced keys. `diagnostic()` is safe to log and preserves opaque backend diagnostics.

## Further reading

- [Parent module (`security`)](../README.md)
- [OS keyring and secret storage](../../../../../docs/gitbooks/en/features/os-keyring-and-secret-storage.md)
- [Security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md)
