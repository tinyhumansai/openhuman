---
description: >-
  How OpenHuman updates itself: one signed manifest, a bounded download retry,
  and a single update that covers the app and its core.
icon: arrows-rotate
---

# Auto-update

OpenHuman updates the whole app in one step. The core is linked into the desktop shell, so there is no separate core update. Updating the shell upgrades both.

## What you see

When a newer build is available, the app downloads it in the background. A prompt appears once the download is ready and moves through **ready to install**, **installing** and **restarting**. If you are current you see **up to date**. A failure says so instead of retrying forever.

Before installing, the app shuts down the in-process core under the same restart lock it uses elsewhere. That way replacing the bundle never races a core that still holds file handles, which keeps a `.app` replacement on macOS clean.

## Where updates come from

There is one endpoint, a manifest published with every release:

```text
https://github.com/tinyhumansai/openhuman/releases/latest/download/latest.json
```

Every artifact is signed, and the public key is compiled into the app. A build whose signature does not verify is refused. There is no update server to trust and no second channel.

## Retries

A download gets three attempts: one try plus two retries, with a delay of two seconds times the attempt number.

Only transient network failures are retried. Signature failures, filesystem errors and "no artifact for this target" are not. Downloading again cannot fix a bad signature, and looping on a failed verification would waste bandwidth and could hide tampering. The policy lives in `crates/openhuman-app/src/app_update.rs`, where `classify` and `is_transient_updater_err` decide.

## Periodic checks

The core runs the schedule, set under `[update]` in `config.toml`:

| Key | Default | What it does |
| --- | --- | --- |
| `enabled` | `true` | Turns periodic checks off when set to `false`. |
| `interval_minutes` | `60` | How often to check. The minimum is 10 minutes. |
| `restart_strategy` | `self_replace` | `self_replace` restarts in place after staging. `supervisor` stages the update and leaves the restart to whatever manages the process. |
| `rpc_mutations_enabled` | `true` | Whether a bearer-authenticated RPC client may call the mutating update methods. |

Use `supervisor` for a headless or containerized core, where something else owns the process lifecycle and the update should not restart it.

## Headless and self-hosted cores

A core you run yourself (`openhuman-core serve`) checks and stages updates through the `update` namespace instead of the desktop prompt. That is why `rpc_mutations_enabled` is a separate switch. The old desktop commands `check_core_update` and `apply_core_update` still exist for frontend compatibility but do nothing.

## See also

- [Platform and availability](../features/platform.md): what ships on each OS.
- [Release policy](../developing/release-policy.md): how a release is cut, and the OAuth minimum-version gate.
- [Cloud deploy](../features/cloud-deploy.md): running the core somewhere else.
