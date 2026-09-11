# keyring

OS-keychain-backed secret storage with pluggable test/debug backends, plus a ChaCha20-Poly1305 encrypted secret store for config-field encryption. This is an **infrastructure domain** — it has no RPC controllers, no agent tools, and no event-bus subscribers. It provides a namespaced, user-scoped, `get`/`set`/`delete` interface over a backend selected once per process, and a separate `SecretStore` type that other domains use to encrypt/decrypt secret values stored in config. All keys are scoped under a `user_id` so multiple users coexist without collision; the backend entry key format is `"{user_id}:{logical_key}"`.

## Responsibilities

- Provide `get` / `set` / `delete` / `get_or_create_random` over a process-global secret-storage backend.
- Select the backend exactly once at first use (env override → `cfg(test)` → staging/prod vs dev) and freeze it in a `OnceLock`.
- Probe and cache backend availability (`is_available`) so callers (wallet guards, snapshot loops) can fall back to file storage without re-triggering OS keychain prompts.
- Migrate a secret from a plaintext file into the active backend (`migrate_from_file`), verifying the write before deleting the source.
- Encrypt/decrypt config-field secrets via `SecretStore` (ChaCha20-Poly1305, `enc2:` prefix), including migration of the legacy XOR `enc:` format.
- Load/cache a single app-scoped master encryption key from the OS keychain at startup (`init_master_key`) for the `encrypted_file` backend, reducing keychain access to one call per process.
- Migrate the legacy plaintext `dev-keychain.json` into the encrypted `secrets.enc` file on first use.

## Key files

| File                                                       | Role                                                                                                                                                                                                                                                                                            |
| ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/openhuman/security/keyring/mod.rs`                    | Module docstring + exports only. Documents backend-selection priority and the Linux-headless availability note. Re-exports the public surface.                                                                                                                                                  |
| `src/openhuman/security/keyring/ops.rs`                    | Core operations: `get`/`set`/`delete`/`is_available`/`get_or_create_random`/`migrate_from_file`, the `MigrationOutcome` enum, `namespaced_key` helper, cached availability probe, and `force_backend_for_test`.                                                                                 |
| `src/openhuman/security/keyring/store.rs`                  | Backend selection + global state. Owns `WORKSPACE_DIR` and `BACKEND` `OnceLock`s, `init_workspace`, `backend()`, `build_backend()`/`build_backend_at()`, and `workspace_dir_for_file_backend()` path derivation. Also the `cfg(test)`-only `test_scope` module (thread-scoped test workspaces). |
| `src/openhuman/security/keyring/backend.rs`                | `KeyringBackend` trait + `OsBackend` (native keychain via the `keyring` crate, service name `"openhuman"`), `FileBackend` (plaintext `dev-keychain.json`, test/debug only), and test-only `MockBackend`.                                                                                        |
| `src/openhuman/security/keyring/encrypted_file_backend.rs` | `EncryptedFileBackend` — all secrets in one ChaCha20-Poly1305 `secrets.enc` file keyed by an app master key; `init_master_key`/`is_master_key_available`; legacy `dev-keychain.json` migration; corrupt-file quarantine.                                                                        |
| `src/openhuman/security/keyring/encrypted_store.rs`        | `SecretStore` — config-field encryption (`enc2:` ChaCha20-Poly1305, legacy `enc:` XOR migration), keychain-backed master key with legacy `.secret_key` file migration, process-wide key cache, Windows ACL repair (`icacls`).                                                                   |
| `src/openhuman/security/keyring/file_store.rs`             | Shared secrets-file primitives for both file backends: the cross-process advisory write lock (`lock_for_write`, on a sidecar `<path>.lock`), the `0600` unique-temp-then-rename `write_atomic`, and `quarantine_corrupt`.                                                                      |
| `src/openhuman/security/keyring/crypto.rs`                 | Shared ChaCha20-Poly1305 helpers (`chacha20_encrypt`/`chacha20_decrypt`), random-byte generation, hex encode/decode. Used by both `encrypted_store` and `encrypted_file_backend`.                                                                                                               |
| `src/openhuman/security/keyring/error.rs`                  | `KeyringError` (thiserror) with variants `Os`/`InvalidUtf8`/`MigrationReadFailed`/`VerifyFailed`/`MigrationDeleteFailed`/`RandomGeneration`/`Crypto`/`Backend`, plus a log-safe `diagnostic()` that preserves the `keyring::Error` variant + `OSStatus`.                                        |
| `src/openhuman/security/keyring/tests.rs`                  | Module tests (backend isolation via `force_backend_for_test`).                                                                                                                                                                                                                                  |
| `src/openhuman/security/keyring/store_tests.rs`            | Test-isolation regressions: test builds ignore `OPENHUMAN_WORKSPACE`, production resolution still honours it, scoped workspaces do not share secrets, and a deleted scoped workspace cannot reset the default store.                                                                            |
| `src/openhuman/security/keyring/encrypted_store_tests.rs`  | `SecretStore` tests (wired via `#[path]` from `encrypted_store.rs`).                                                                                                                                                                                                                            |

## Public surface

Re-exported from `mod.rs`:

- `KeyringBackend` — backend trait (`get`/`set`/`delete`/`name`).
- `SecretStore` — config-field encrypt/decrypt; `encrypt`/`decrypt`/`decrypt_and_migrate`/`needs_migration`/`is_encrypted`/`is_secure_encrypted`/`new`.
- `KeyringError` — error enum with `diagnostic()`.
- `init_master_key` — load the app master key from the OS keychain at startup (staging/prod only).
- `init_workspace` — register the workspace dir for file/encrypted-file backends.
- `get`, `set`, `delete`, `get_or_create_random`, `is_available`, `migrate_from_file`, `MigrationOutcome`.
- `force_backend_for_test` — `pub(crate)`, test-only.

## RPC / controllers

None. The keyring domain exposes no `schemas.rs`, no `all_*_controller_schemas`, and no `openhuman.keyring_*` methods. It is consumed in-process by other domains.

## Agent tools

None.

## Events

None — no `bus.rs`, publishes/subscribes to no `DomainEvent`.

## Persistence

Secret storage backend, selected once and frozen in a `OnceLock`:

- **`os`** (production default outside staging/prod special-casing): native OS credential store — macOS Keychain, Windows Credential Manager, Linux Secret Service — under service name `"openhuman"`.
- **`encrypted_file`** (staging/production, and via `OPENHUMAN_KEYRING_BACKEND=encrypted_file`): single ChaCha20-Poly1305 file `{workspace}/secrets.enc`, encrypted with a master key loaded once from the OS keychain (`openhuman` / `app:master_key`). Files written `0600` on Unix via temp-file + atomic rename.
- **`file`** (dev default, `cfg(test)`, or `OPENHUMAN_KEYRING_BACKEND=file`): plaintext JSON `{workspace}/dev-keychain.json`. **Not encrypted — test/debug only.**
- **`mock`** (test-only): in-memory `HashMap`.

`SecretStore` additionally manages a master encryption key: keychain-backed (slot `secretstore.master_key`) in normal builds with one-time migration from the legacy `{data_dir}/openhuman/.secret_key` file; the file path is retained only for unit tests. Decoded keys are cached process-wide keyed by normalized path.

Workspace dir resolves from `init_workspace`, else `OPENHUMAN_WORKSPACE`, else `~/.openhuman` (or `~/.openhuman-staging` under `OPENHUMAN_APP_ENV=staging`). **In `cfg(test)` builds only**, that rule is bypassed — see the test-isolation note below.

Both file backends keep every secret in one file, so a `set` of one key rewrites all of them. That read-modify-write cycle is guarded by `file_store::lock_for_write` — an in-process mutex is not sufficient, because a desktop core, a `medulla` TUI embedding the same core, and a `cargo test` run that inherited `OPENHUMAN_WORKSPACE` all address the same path.

## Dependencies

Internal openhuman/core modules: **none** — the keyring module's own files only `use crate::openhuman::security::keyring::*` (self-internal). It is a leaf infrastructure module. External crates: `keyring`, `chacha20poly1305`, `serde_json`, `parking_lot`, `thiserror`, `anyhow`, `chrono`, `dirs`.

## Used by

Discovered consumers (`crate::openhuman::security::keyring::*`):

- `src/lib.rs` and `src/core/jsonrpc.rs` — call `init_master_key()` at startup.
- `src/openhuman/security/secrets.rs`, `src/openhuman/security/mod.rs` — secret handling.
- `src/openhuman/config/schema/load.rs` — `SecretStore::new` / `is_encrypted` to encrypt/decrypt config fields on load.
- `src/openhuman/security/credentials/profiles.rs`, `credentials/ops.rs` — `SecretStore`, `is_available`, `get`/`set`/`delete` for per-profile credential storage.
- `src/openhuman/web3/wallet/ops.rs` — `is_available`/`get`/`set` for the wallet mnemonic.
- `src/openhuman/security/devices/rpc.rs` — device secret handling.

## Notes / gotchas

- **Backend is frozen on first use (production).** Selection order: `OPENHUMAN_KEYRING_BACKEND` (`os`/`file`/`encrypted_file`) → `cfg(test)` → `file` → staging/prod → `encrypted_file`, dev → `file`. Once `BACKEND` is set it cannot change for the process lifetime. A process serves one workspace, so this `OnceLock` is a pure cache.
- **Test builds resolve the workspace per thread, not per process.** `WORKSPACE_DIR` and `OPENHUMAN_WORKSPACE` are shared by every concurrently-running test, so consulting them made the whole test binary share one credential store pinned to whichever workspace won the race. When the winner was a `TempDir` held by a test's env guard, that directory was deleted at the end of that test and — because `FileBackend::read_map` treats a missing file as an empty map — the next write silently reset the store, so unrelated tests read their own freshly-written secrets back as `None`. Under `cfg(test)`, `workspace_dir_for_file_backend()` therefore ignores both and uses `test_scope::current_workspace()`: a thread-local override when a test binds one with `test_scope::ScopedWorkspace`, otherwise a stable per-process directory under the system temp dir. Backends are then cached **per resolved directory**. Two consequences worth knowing: test runs never read or write the developer's real `~/.openhuman/dev-keychain.json`, and a test wanting a private credential store must use `ScopedWorkspace` — setting `OPENHUMAN_WORKSPACE` no longer steers the keyring. Production selection is unchanged.
- **Cleaning up historical leakage:** `node scripts/prune-dev-keychain.mjs` reports (and with `--apply` removes, after taking a backup) `dev-keychain.json` entries keyed by dead `TempDir` basenames, left behind before the isolation fix. Dry run by default.
- **`is_available()` is cached after the first probe.** The probe performs delete/set/get/delete round-trips on the `os` backend; running it per-call triggered repeated macOS permission dialogs and starved frequent pollers. Non-os backends short-circuit to `true`. A failed probe is logged at `warn` because it silently flips `use_keychain` off.
- **`is_available()` answers "is the active backend usable", never "are secrets in the OS keychain".** Because the file backends short-circuit to `true`, availability says nothing about *where* secrets are — which is why `keyring_consent::policy::active_mode_for` derives `active_mode` from `backend_name()` and consults availability only for the `os` backend. Deriving it from `is_available()` alone is #6076: the shipped staging/production app told users their secrets were in the OS keychain while they were in `{workspace}/secrets.enc`. Consumers that genuinely want the usability answer (wallet guards, snapshot loops) read `available` and are unaffected.
- **`force_backend_for_test` panics if `BACKEND` is already initialized** — it must run before any keyring call in the same process (dedicated test binary or very top of a test).
- **`migrate_from_file` never deletes the source unless the verified write succeeds**, so failures are retryable.
- **`encrypted_file` corrupt/undecryptable files are quarantined** (renamed `secrets.enc.corrupt.<ts>`) and treated as empty rather than crashing.
- **A corrupt file is never *overwritten*.** `file` reads degrade to an empty map (so a missing token means "sign in again"), but a `set`/`delete` over an unparseable file quarantines it and returns an error. Returning empty on the write path is what turned a corrupt file into a wipe: the write that followed persisted a map holding nothing but the key being set.
- **Mutations hold a cross-process lock**, on `<secrets file>.lock` rather than the secrets file itself — `write_atomic` replaces the file by rename, so a lock on the old inode would guard nothing. Callers must hold it across the read *and* the write.
- **`SecretStore` master-key file is write-once**; on Windows it survives transient AV-scanner sharing violations via retry/backoff and attempts `icacls` ACL self-repair on permission errors. Decoded keys are cached so repeated decrypts (e.g. snapshot polls) hit memory.
- **Legacy formats:** `SecretStore` migrates `enc:` (XOR) → `enc2:` (ChaCha20-Poly1305) on decrypt; `EncryptedFileBackend` migrates plaintext `dev-keychain.json` → `secrets.enc` (renaming the legacy file `.json.migrated`).
- **Errors never carry secret values** — only namespaced keys; `diagnostic()` is safe to log and preserves the underlying `keyring::Error` variant/`OSStatus`.
