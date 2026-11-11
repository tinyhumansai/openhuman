---
description: >-
  How OpenHuman uses the OS keyring to protect local secrets, what stays
  encrypted on disk, and what the consent prompt asks.
icon: key
---

# OS keyring and secret storage

The OpenHuman desktop app uses your operating system's secure credential store to protect the master key for local secrets:

- **macOS:** Keychain
- **Windows:** Credential Manager
- **Linux:** Secret Service (libsecret)

This is the root of trust for local secrets. OpenHuman does not keep user credentials in a plaintext `.env` file or a plaintext config file.

## What goes into the keyring

The keyring holds two kinds of secret.

**Credential entries.** In the desktop app, locally stored provider API keys, session and bearer tokens, and wallet secret material use user-scoped credential slots. By default, these slots live in the encrypted `secrets.enc` file, protected by the master key in the OS keyring. The explicit `OPENHUMAN_KEYRING_BACKEND=os` setting stores the slots directly in the platform keyring, under OpenHuman's own namespace.

**The master encryption key.** Some sensitive values must live inside local files, because the app's configuration is file-based. OpenHuman splits the storage. The secret value is stored on disk as encrypted ciphertext. The master key that decrypts it lives in the OS keyring. Your config and state files can hold encrypted values without the decryption key sitting next to them.

## What stays encrypted on disk

When OpenHuman saves a sensitive setting locally, it writes the ciphertext to disk and keeps the key in the keyring. That covers:

- BYO API keys for supported providers.
- Channel and webhook secrets stored in local config.
- Other locally saved secret settings that desktop features need.

The encryption is authenticated, so OpenHuman detects tampering instead of silently accepting changed ciphertext. In short: the key is in the keyring, the ciphertext is in the file, and plaintext exists only in memory when needed.

## Why this beats plaintext config

Plaintext secrets in config files are a risk if you have a workspace backup, a sync folder or a support bundle. With the keyring as the root secret store:

- You can copy config files without exposing raw credentials.
- Accidental log or file inspection is less likely to reveal secrets.
- The decryption key belongs to the platform's credential system, not to a plaintext file the app manages.

This does not replace full-disk encryption or OS account security. It is a narrower, stronger way to handle application secrets.

## Managed integrations and local secrets

In the default managed integration flow, the OpenHuman backend handles third-party OAuth tokens. Your app does not need to keep those provider tokens in plaintext on your machine.

When you choose a bring-your-own-key or direct-mode path, OpenHuman treats those credentials as local secrets. It protects them with the OS keyring and encrypted local storage where needed.

## Migrating from older installs

Older versions could keep local encryption material in a file. Current desktop builds move that material into the OS keyring and keep the encrypted values on disk. You do not need to re-enter your secrets.

## When the keyring is unavailable

Sometimes the keyring cannot be reached. On Linux that can mean no Secret Service daemon. On macOS it can mean keychain access was denied. When that happens, OpenHuman stops and asks before it falls back to local encrypted storage.

1. **Detection.** On startup the core probes the OS keychain. If the probe fails, it classifies the reason (no daemon, locked, denied) and reports a structured `KeyringStatus` through the `openhuman.keyring_consent_status` RPC and the app snapshot.
2. **Consent prompt.** The first time a secret must be read or written without recorded consent, a modal explains what happened, what "store locally" means and the risks. You can choose:
   - **Use Local Encrypted Storage:** consent to ChaCha20-Poly1305 encrypted files. The master key is also on disk.
   - **Retry OS Keychain:** probe again, which helps after you grant OS permission.
   - **Skip:** decline local storage. Features that need secrets will be unavailable.
3. **Saved choice.** Your choice is recorded in `app-state.json` (the `keyringConsent` field) and cached in the process. The app probes again on each launch and asks again if the keyring becomes available after a local-only session.
4. **Settings.** **Settings → Security** shows the active storage mode, keychain availability and failure reason, with buttons to retry or change consent.

### One fallback policy

Auth profiles, config secrets, the wallet mnemonic and the `secrets.enc` backend all call `keyring_consent::policy::check_secret_access()` instead of checking `is_available()` directly. No code path switches storage modes silently.

| Policy decision | Meaning |
| --- | --- |
| `Proceed` | The OS keyring is available, or you consented to local encrypted storage. |
| `ConsentRequired` | The keyring is unavailable and there is no consent yet. Block and prompt. |
| `Declined` | You refused local storage. Skip the secret operation. |

## Embedded and server hosts

Desktop, CLI and TUI hosts install the OS keychain providers through
`openhuman-rpc`'s `os-keyring` feature before starting the core. The service
name (`openhuman`) and master-key account (`app:master_key`) are unchanged,
so existing desktop workspaces continue to decrypt with the same key.

The core owns the platform-independent `security::secret_storage` module:
user-scoped secret access, encrypted files, and master-key resolution. RPC owns
`keyring`, the native OS providers, and installs them through core's `SecretBackend`
and `MasterKeySource` contracts.

The core and embedded library default to `MemoryBackend`: a thread-safe,
unencrypted map held only in the process. It needs no OS keychain, master key,
or secrets file. Secrets disappear when the process exits. Use it for ephemeral
library runtimes; choose persistent storage when credentials must survive a restart.

Embedders can install a provider before boot through
`openhuman_embed::secret_storage::install_secret_backend(Arc::new(provider))`.
The same facade exposes `MemoryBackend`, `EncryptedFileBackend`, `SecretBackend`
and `SecretStorageError`. The provider is process-wide, matching the one-runtime
contract; install it before building the runtime.

CLI, desktop and TUI hosts explicitly retain encrypted-file storage by default.
With native support enabled, its master key stays in the OS keychain. A host
without that support creates `{workspace}/keyring/master.key` instead (Unix mode
`0600`). Back up that key with the workspace.

`OPENHUMAN_KEYRING_BACKEND=memory` selects the basic volatile backend explicitly;
`file` selects the plaintext debug file, `encrypted_file` selects persistent
encrypted storage, and `os` requires RPC's native provider. Installed providers
win over environment selection.

For encrypted storage, an operator can supply `OPENHUMAN_KEYRING_MASTER_KEY`
(64 hex characters) or `OPENHUMAN_KEYRING_MASTER_KEY_FILE`. Set exactly one.
Environment keys take precedence over a host source, then the workspace key file.
An inaccessible host key never falls back to a newly created file key.

### Moving a desktop workspace to a server

A desktop workspace encrypted with an OS keychain key needs that same key
when opened by a host without OS keychain support. Supply it through the
master-key environment or file option before boot. A newly generated
workspace key cannot decrypt the desktop ciphertext. Unreadable
`secrets.enc` files are quarantined as `secrets.enc.corrupt.<timestamp>`
for recovery rather than overwritten; keep the original key and backups.

## See also

- [Privacy and security](privacy-and-security.md)
- [Third-party integrations](integrations/README.md)
- [Local AI (optional)](model-routing/local-ai.md)
