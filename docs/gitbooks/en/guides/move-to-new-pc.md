---
description: >-
  Carry your OpenHuman persona, workspace and model config to a new computer,
  and learn which secrets travel and which you re-enter.
icon: truck
---

# Move OpenHuman to a new PC

This guide sets up OpenHuman on a new machine so it picks up where the old one left off, with the same persona and settings and your memory reconnected. In short, copy one folder and sign back in. The details below cover what a folder copy does and does not carry.

## Prerequisites

- Both computers, or a backup of the old one's data folder.
- Your OpenHuman sign-in credentials.
- A way to move files between them, such as an external drive or a secure file transfer.

## What lives where

Everything OpenHuman keeps on disk is in one folder:

| Platform | Data folder |
| --- | --- |
| macOS and Linux | `~/.openhuman/` |
| Windows | `%USERPROFILE%\.openhuman\` |

These are the parts you care about moving:

| What | Where | Travels with a folder copy? |
| --- | --- | --- |
| Memory items | In your engine (TinyHumans or CortexDB) | No. Sign in again or re-enter the key |
| Persona and behavior | `SOUL.md`, `IDENTITY.md`, `ROLE.md` | Yes |
| Config (models, providers, routing, autonomy) | `config.toml` | Yes |
| Session history | `sessions/`, `session_raw/` | Yes |
| Approval history | `approval/approval.db` | Yes |
| OS-stored secrets (session token, some local keys) | Your OS keychain, not this folder | No. Re-established on sign-in |
| Integration access (Gmail, Slack, and so on) | Held by the backend against your account | No. Reconnects on sign-in |

{% hint style="info" %}
OpenHuman keeps secrets out of loose files on purpose. Your session token and some local secrets live in the operating system's secure store (Keychain, Credential Manager or Secret Service), and the backend holds your integration tokens against your account. The folder copy carries your data and persona, and signing in on the new machine restores the secrets and integrations. You never copy raw tokens by hand.
{% endhint %}

## Steps

### 1. Quit OpenHuman on the old machine

Close the app fully so nothing is mid-write to the database. A clean copy needs a quiet source.

### 2. Copy the data folder

Copy the entire data folder from the old machine to the same location on the new one:

- macOS and Linux: copy `~/.openhuman/` to `~/.openhuman/`.
- Windows: copy `%USERPROFILE%\.openhuman\` to `%USERPROFILE%\.openhuman\`.

Copy the whole folder instead of picking files. That keeps persona, config and history consistent with each other.

The data folder holds config but not the files the agent created or edited in its action sandbox. Also copy your projects folder, by default `~/OpenHuman/projects`, or wherever you pointed the action directory. Otherwise those project files stay on the old PC. That folder also holds `Files` (`~/OpenHuman/projects/Files`), where the agent saves the decks, documents, images and videos it delivers.

{% hint style="warning" %}
Copy it somewhere secure. The folder contains your personal data in readable form, so treat the transfer like moving personal documents.
{% endhint %}

### 3. Install OpenHuman on the new machine

Install the current build from [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman) or the [latest release](https://github.com/tinyhumansai/openhuman/releases/latest). If the data folder is already in place, the app finds it on launch. The order does not matter much. You can install first and copy after, as long as the app is not running during the copy.

### 4. Launch and sign in

Open the app and sign in with the same account. Signing in:

- Re-establishes your session token in the new machine's OS keychain.
- Reconnects your account, so integrations held by the backend come back.

### 5. Reconnect anything tied to your account

- **Integrations** (Gmail, Slack and so on): confirm they show as connected under **Settings**. If one needs a fresh OAuth approval, approve it again. It takes one click.
- **Your own keys:** if you entered your own provider API key, a Composio direct key or similar local secrets, enter them again. They live in the OS keychain and do not travel in the folder.

### 6. Re-check model and provider config

Your `config.toml` came along, so model routing and provider choices should already match. If you used a [local model](local-model.md), remember that Ollama or LM Studio is separate software, and the model weights live in its own store, not in the OpenHuman data folder. Install the runtime on the new machine and pull the same models yourself, for example `ollama pull bge-m3`. OpenHuman does not download them.

## Success checks

The move worked when:

- [ ] The Memory tab on the new machine shows your existing summaries.
- [ ] The assistant replies in your configured style, and your display name and persona are intact.
- [ ] Connected integrations show as connected under **Settings**. Reconnect any that do not.
- [ ] Your autonomy tier and settings match what you had (check **Settings → Agents → Agent access**).
- [ ] If you use local AI, the runtime is installed and running, you have pulled the models your workloads name, and a turn routed to the local provider answers.

## Common failures

| Symptom | Cause | Fix |
| --- | --- | --- |
| The new machine starts fresh with no memory | The data folder was in the wrong place, or the app was running during the copy | Quit the app, place the folder at `~/.openhuman/` (or `%USERPROFILE%\.openhuman\`), and relaunch |
| Signed in but integrations are disconnected | Integration access is tied to your account, not the folder | Reconnect each integration in Settings. Each takes one OAuth click |
| The local model does not work on the new PC | Ollama or LM Studio and the weights are not on the new machine | Install the runtime and pull the models yourself. See the [local model guide](local-model.md) |
| The assistant lost its personality | `SOUL.md` and `IDENTITY.md` were not copied | Copy the whole data folder, not just the database |
| Sign-in stalls on the new machine | An auth or handler issue unrelated to the move | See [Troubleshooting sign-in](../overview/troubleshooting-sign-in.md) |

## Recovery

- Keep the old machine's folder until you have verified the new one. Do not wipe the source until every success check passes.
- If the new machine will not start at all, treat it as a fresh-install problem and follow [Recover from a failed installation](recover-failed-installation.md). Your copied folder is safe to move aside and restore.

## See also

- [Recover from a failed installation](recover-failed-installation.md): the same data folder, a different problem.
- [Keep sensitive data private](privacy-sensitive-data.md): why secrets are stored the way they are.
- [Use OpenHuman with a local model](local-model.md): setting up a runtime on the new machine.
- [OS keyring and secret storage](../features/os-keyring-and-secret-storage.md): what the keychain holds instead of the folder.
