# Secret Key Keychain Migration

## Goal

Move the `SecretStore` master encryption key out of `.secret_key` files and into the OS keychain for real app environments.

Accepted exception:
- unit tests and explicit debug overrides may keep using file-backed storage.

## Current State

- Auth/provider credentials already prefer the keychain when available.
- `SecretStore` still keeps its master key in `{openhuman_dir}/.secret_key`.
- `keyring` currently falls back to a file backend in some non-test environments.

## Target State

- `dev`, `staging`, and `prod` use the OS keychain for the `SecretStore` master key.
- Existing ciphertext stays on disk unchanged.
- Existing `.secret_key` files are migrated into the keychain and then deleted only after verification.
- Unit tests continue to work without depending on the host OS keychain.

## Migration Plan

1. Change keyring backend selection so `cfg(test)` keeps the file backend, but normal app environments default to the OS backend unless explicitly overridden.
2. Teach `SecretStore` to:
   - derive a stable keychain namespace from the user/openhuman directory
   - migrate an existing `.secret_key` file into the keychain
   - create the key in keychain when no legacy file exists
   - fall back safely when keychain is unavailable
3. Add tests for:
   - legacy `.secret_key` migration
   - post-migration decrypt compatibility
   - unit-test file-backed behavior

## Constraints

- Never re-encrypt existing payloads unless the ciphertext format itself changes.
- Never delete `.secret_key` until the keychain write is verified.
- Keep an explicit override path for debugging and recovery.
