# keyring_consent

Gates the fallback from the OS keyring to local encrypted storage behind
explicit user consent. All code that reads or writes secrets should call
`policy::check_secret_access()` instead of `secret_storage::is_available()` directly,
so the app never silently drops to local storage without the user agreeing.

## Responsibilities

- Decide, from backend identity and cached consent, whether secret access may
  proceed (`PolicyDecision::Proceed` / `ConsentRequired` / `Declined`).
- Report `KeyringStatus` (`available`, `failure_reason`, `active_mode`,
  `backend_name`) for the settings UI.
- Persist and cache the user's consent decision (`ConsentPreference`).
- Publish `DomainEvent::KeyringConsentRequired` /
  `KeyringDecryptFailed` when the frontend needs to prompt or warn the user.

## Key files

| File | Purpose |
| --- | --- |
| [`types.rs`](./types.rs) | `StorageMode`, `KeyringFailureReason`, `KeyringStatus`, `ConsentPreference`, `PolicyDecision` |
| [`policy.rs`](./policy.rs) | `check_secret_access`, `current_status`, `active_mode_for`, `record_consent`/`build_consent_preference`/`apply_consent`, `retry_probe`, `notify_master_key_unavailable`, `notify_decrypt_failure` |
| [`ops.rs`](./ops.rs) | RPC handler bodies: `keyring_status`, `keyring_consent_decide`, `keyring_retry_probe` |
| [`schemas.rs`](./schemas.rs) | Controller registration for the `keyring_consent` RPC namespace |
| [`mod.rs`](./mod.rs) | Module doc + re-exports |

## Public surface

- `StorageMode`: where secrets actually live. `OsKeyring` (no consent
  needed); the consent outcomes `LocalEncrypted` / `ConsentPending` /
  `Declined` (only reachable when the `os` backend was intended and could not
  be used); and the operator-configured backends `LocalEncryptedFile`
  (`encrypted_file`) / `LocalPlaintextFile` (`file`/`mock`), for which no
  consent was ever asked. The last two groups are kept distinct from
  `LocalEncrypted` because they are not the same storage; see the type's
  doc comment.
- `KeyringStatus`: `available`, `failure_reason: Option<KeyringFailureReason>`,
  `active_mode: StorageMode`, `backend_name: String`. `active_mode` is derived
  from `backend_name` first and availability second (`policy::active_mode_for`).
  Deriving it from `available` alone previously made every non-`os` backend
  report `OsKeyring` (#6076).
- `ConsentPreference { storage_mode, consented_at_ms }`: the persisted
  decision.
- `PolicyDecision::{Proceed, ConsentRequired, Declined}`: returned by
  `check_secret_access()`.
- `policy::check_secret_access() -> PolicyDecision`: the gate every secret
  read/write path should call. Returns `Proceed` immediately if
  `secret_storage::is_available()`; otherwise consults the cached consent
  (`local_encrypted` → `Proceed`, `declined` → `Declined`, none → publishes
  `KeyringConsentRequired` once and returns `ConsentRequired`).
- `policy::current_status() -> KeyringStatus`: snapshot for RPC/UI.
- `policy::record_consent(mode)` / `build_consent_preference` +
  `apply_consent`: record a decision; the RPC handler persists to disk
  *before* updating the in-memory cache so cache and disk never diverge on a
  failed persist.
- `policy::retry_probe()`: resets the keyring availability cache and
  re-evaluates status; only the `os` backend can change here, since file
  backends always probe available.

## RPC / controllers

Namespace `keyring_consent`, three functions ([`schemas.rs`](./schemas.rs)):

- `keyring_consent.status`: current `KeyringStatus`.
- `keyring_consent.decide`: record `mode` (`"local_encrypted"` or
  `"declined"`); persists via `desktop::app_state::update_local_state` then
  updates the cache.
- `keyring_consent.retry_probe`: reset and re-run the OS keyring probe.

## Persistence

The consent decision is persisted through
`desktop::app_state::StoredAppStatePatch { keyring_consent, .. }` /
`update_local_state` (app-state snapshot), and mirrored into a process-wide
in-memory cache (`policy::CONSENT_CACHE`) so `check_secret_access` never
touches disk on the hot path. `policy::initialize` re-populates the cache from
a loaded app-state snapshot; it is change-gated so repeated snapshots with an
unchanged value are a silent no-op.

## Relates to

[`../keyring/README.md`](../keyring/README.md) owns backend selection
(`os` / `encrypted_file` / `file` / `mock`) and the actual secret storage
(`get`/`set`/`delete`, `SecretStore`, `encrypted_file_backend.rs`). This
domain never stores a secret itself. It only gates and reports on *which*
backend is active and whether the user consented to it. `active_mode_for`
matches on `secret_storage::backend_name()`'s identifiers (`"os"`,
`"encrypted_file"`, `"file"`, `"mock"`) to answer that question.

## Used by

- `credentials/profiles/keychain.rs` and `credentials/credential_ref.rs`:
  consent preflight before profile secrets and credential refs touch the
  keyring.
- `web3/wallet/ops/state.rs`: mnemonic goes to the keychain only on
  `Proceed`.
- `desktop/app_state/`: carries `keyring_consent` in the stored app state and
  `keyring_status` in the snapshot; `policy::initialize` is fed from there.

## Tests

- [`types_tests.rs`](./types_tests.rs), [`policy_tests.rs`](./policy_tests.rs), [`ops_tests.rs`](./ops_tests.rs), [`schemas_tests.rs`](./schemas_tests.rs).

## Further reading

- [Parent module (`security`)](../README.md)
- [OS keyring and secret storage](../../../../../docs/gitbooks/en/features/os-keyring-and-secret-storage.md)
- [Security architecture](../../../../../docs/gitbooks/en/developing/architecture/security.md)
