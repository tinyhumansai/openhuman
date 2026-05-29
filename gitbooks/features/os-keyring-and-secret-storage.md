---
icon: key
---

# OS Keyring & Secret Storage

OpenHuman uses the **operating system's secure credential store** to protect the secrets that must live on your device.

On desktop builds, that means:

* **macOS:** Keychain
* **Windows:** Credential Manager
* **Linux:** Secret Service / libsecret

This is the root of trust for local secret material. OpenHuman does not rely on a plaintext `.env` file or a plaintext local config file for user credentials.

***

## What goes into the OS keyring

OpenHuman uses the OS keyring for two kinds of local secret material:

### 1. Credential entries

When a feature needs a local credential slot, OpenHuman stores it in the platform keyring rather than writing the raw secret into a normal config file.

Examples include:

* locally stored provider API keys
* session and bearer tokens that must remain on-device
* wallet secret material where applicable

These entries are scoped under OpenHuman's own key namespace so they do not collide with unrelated apps.

### 2. The master encryption key

Some sensitive values still need to live **inside local files** because the application configuration itself is file-based.

OpenHuman handles that by splitting storage in two:

* the **secret value on disk** is stored as encrypted ciphertext
* the **master key used to decrypt it** lives in the OS keyring

This means your local config and state files can contain encrypted values without the decryption key sitting beside them in plaintext.

***

## What stays encrypted on disk

When OpenHuman needs to persist sensitive application settings locally, it writes the **ciphertext** to disk and keeps the key in the OS keyring.

That covers local secrets such as:

* BYO API keys for supported providers
* channel and webhook secrets stored in local config
* other locally persisted secret settings required for desktop features

The encryption format is authenticated, so OpenHuman can detect tampering instead of silently accepting modified ciphertext.

In practice, the security model is:

* **key in keyring**
* **ciphertext in file**
* **plaintext only in memory when needed**

***

## Why this is better than plaintext config

If your machine has a local workspace backup, sync folder, or support bundle, plaintext secrets in config files are a liability.

Using the OS keyring as the root secret store gives OpenHuman a safer split:

* config files can be copied without exposing raw credentials
* accidental log or file inspection is less likely to reveal secrets
* the decryption key is delegated to the platform's credential system rather than to an app-managed plaintext file

This is not a replacement for full-disk encryption or OS account security. It is a narrower, stronger way to handle application secrets.

***

## Managed integrations vs local secrets

Not every secret follows the same path.

### Managed integrations

For the default managed integration flow, third-party OAuth tokens are handled by the OpenHuman backend. Your local app does **not** need to persist those provider tokens in plaintext on your machine.

### Local BYO credentials

When you choose a bring-your-own-key or direct-mode path, OpenHuman treats those credentials as **local secrets** and protects them using the OS keyring plus encrypted-at-rest local storage where needed.

***

## Migration from older installs

Older versions could keep local encryption material in a file-based form.

Current desktop builds migrate that material into the OS keyring and keep the encrypted payloads on disk. The goal is to move the root secret out of ordinary files and into the platform credential store, without requiring users to re-enter every secret by hand.

***

## Platform note

This page describes **desktop** OpenHuman: the Tauri app on macOS, Windows, and Linux.

In development and test environments, the repository may use test-specific overrides so automated runs do not depend on an interactive OS keychain. That is a developer convenience, not the end-user desktop security model.

***

## See also

* [Privacy & Security](privacy-and-security.md)
* [Third-party Integrations](integrations/README.md)
* [Local AI (optional)](model-routing/local-ai.md)
