---
description: >-
  Get a broken or half-installed OpenHuman running again without losing your
  memory, personas or settings.
icon: life-ring
---

# Recover from a failed installation

Use this guide when the app will not install, will not start, or starts broken, and you want it working again without wiping your data.

Your configuration and memory are preserved by default. Recovery means fixing the app around your data, not deleting the data. The only step that touches your data folder is a last resort, and it keeps a backup.

## Prerequisites

- Nothing special. You need the app, a file manager and, occasionally, a terminal.
- Know where your data lives. Everything OpenHuman keeps is in one folder:

  | Platform      | Data folder                 |
  | ------------- | --------------------------- |
  | macOS / Linux | `~/.openhuman/`             |
  | Windows       | `%USERPROFILE%\.openhuman\` |

  Leave that folder alone unless a step here explicitly says to touch it.

## Privacy implications

- Recovery is local. You restart or reinstall software, and your memory is not uploaded.
- If you file a bug report, redact secrets. Share status codes, app version, OS and log lines, never tokens or JWTs.

## First: read the logs

Almost every failure names itself in the log, so start there.

- In the app, if it opens: **Settings → About → App Logs Folder** reveals the folder in your file manager.
- On disk: logs are under your data folder, for example `~/.openhuman/logs/openhuman.<date>.log` (Windows: `%USERPROFILE%\.openhuman\logs\openhuman.*.log`). They rotate daily.

Open the most recent log, read the last error lines, and match the message to the table below.

## Recovery ladder

Work from the top and stop as soon as it works. Each rung is more disruptive than the last, and the early rungs never touch your data.

### Rung 1: Restart cleanly

- Quit OpenHuman fully, make sure no process is left running, and reopen it.
- Run only one instance at a time. A second copy can hold a lock the first needs.

### Rung 2: Reinstall the app over your data

Reinstalling the application does not delete your data folder, because they are separate. This fixes a corrupted or partial install and keeps everything.

- Download the current build from [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman) or the [latest release](https://github.com/tinyhumansai/openhuman/releases/latest) and install over the top.
- On macOS, install the real `.app` bundle. Some features need the bundle, not a dev build.
- Reopen the app. Your memory, personas and settings are still there, because they live in `~/.openhuman/`.

### Rung 3: Fix the specific error

Match your symptom to a fix:

| Symptom in logs / UI                                                                                                   | Cause                                                                        | Fix                                                                                                                                   |
| ---------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| Sign-in stalls after the provider step; log mentions `openhuman://` scheme not registered (Windows)                | The URL handler didn't register, or the install was moved after first launch | Follow the repair steps in [Troubleshooting sign-in](../overview/troubleshooting-sign-in.md#windows-openhuman-handler-not-registered) |
| App won't render, or the window opens blank                                                                           | A previous instance is still holding the port or the webview data directory  | Ensure no other OpenHuman is running; if it persists, close all instances and relaunch                                                |
| Local AI / Ollama errors                                                                                               | The local runtime you configured isn't running, or a model isn't pulled     | This does not block the app. OpenHuman does not install or start the runtime, so start it yourself. See [Use OpenHuman with a local model](local-model.md#common-failures) |
| "Low disk space" warning, or writes failing                                                                            | The workspace can't be written                                               | Free up space (the app wants a healthy margin, a few hundred MB minimum) and restart                                                  |

### Rung 4: Move the data folder aside (non-destructive reset)

If the app still will not start and you suspect the data folder itself, rename it instead of deleting it. You get a clean start and keep a full backup you can restore.

{% hint style="warning" %}
Quit OpenHuman completely before moving its folder.
{% endhint %}

```bash
# macOS / Linux
mv ~/.openhuman ~/.openhuman.backup-$(date +%Y%m%d)
```

```powershell
# Windows (PowerShell)
Rename-Item "$env:USERPROFILE\.openhuman" ".openhuman.backup"
```

Relaunch. OpenHuman creates a fresh data folder and you sign in again.

- If the fresh start works, the old folder was the problem, and your data is safe in the backup. Copy specific pieces back, such as workspace files like `SOUL.md` or `config.toml`, and test after each.
- If it still fails, the data folder was not the cause. Rename the backup back so you lose nothing, and ask for help (below).

## Success checks

You have recovered when:

- [ ] The app launches to the sign-in or chat screen without crashing.
- [ ] You can sign in and reach your home or chat view.
- [ ] Your Memory tab still shows your existing summaries, which confirms your data survived.
- [ ] The most recent log file shows a clean startup with no repeating error.

## Why your data is safe

Rungs 1 to 3 delete nothing. Rung 4 renames your data folder and never removes it, so even the most aggressive step is reversible. Reinstalling the app never touches `~/.openhuman/`.

## If you are still stuck

Open an issue on [GitHub](https://github.com/tinyhumansai/openhuman) or ask on [Discord](https://guild.tinyhumans.ai). Include:

- App version and OS.
- The last error lines from the log (with tokens and JWTs redacted).
- Which rung you reached and what happened.
- Whether moving the data folder aside changed anything.

## See also

- [Troubleshooting sign-in](../overview/troubleshooting-sign-in.md): the deep dive for auth-specific failures.
- [Move OpenHuman to a new PC](move-to-new-pc.md): the same data folder is what you carry over.
