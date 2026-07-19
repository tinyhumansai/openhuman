---
description: >-
  Get a broken or half-installed OpenHuman running again without losing your
  memory, personas, or settings. Configuration is preserved by default.
icon: life-ring
---

# Recover from a failed installation

**Goal:** the app won't install, won't start, or starts broken, and you want it working again **without wiping your data**.

The guiding rule of this guide: **your configuration and memory are preserved by default.** Recovery means fixing the runtime around your data, not deleting the data. We only touch your data folder as an explicit, backed-up last resort.

***

## Prerequisites

* Nothing special. You can do all of this from the app plus a file manager and, occasionally, a terminal.
* Know where your data lives. **Everything OpenHuman persists is in one folder:**

  | Platform | Data folder |
  | -------- | ----------- |
  | macOS / Linux | `~/.openhuman/` |
  | Windows | `%USERPROFILE%\.openhuman\` |

  Leave that folder alone unless a step here explicitly says to touch it.

## Privacy implications

* Recovery is local. You're restarting or reinstalling software; your memory never gets uploaded as part of this.
* If you file a bug report, **redact secrets**. Attach status codes, app version, OS, and log lines, never tokens or JWTs.

***

## First: read the logs

Almost every failure names itself in the log. Get to the logs first; it turns guessing into fixing.

* **In the app (if it opens):** **Settings → About → App Logs Folder**. There's a button to reveal the folder in your file manager.
* **On disk:** logs are under your data folder, e.g. `~/.openhuman/logs/openhuman.<date>.log` (Windows: `%USERPROFILE%\.openhuman\logs\openhuman.*.log`). They rotate daily.

Open the most recent log and read the last error lines. Match the message to the tables below.

***

## Recovery ladder

Work top to bottom. **Stop as soon as it works.** Each rung is more disruptive than the last, and the early rungs never touch your data.

### Rung 1: Restart cleanly

* Fully quit OpenHuman (make sure no leftover process is running) and reopen it.
* Only **one** instance should run at a time. A second copy can hold a lock the first one needs.

### Rung 2: Reinstall the app over your data

Reinstalling the **application** does not delete your **data folder**; they're separate. This fixes a corrupted or partial install while preserving everything.

* Download the current build from [tinyhumans.ai/openhuman](https://tinyhumans.ai/openhuman) and install over the top.
* On macOS, install the real `.app` bundle (some features need the bundle, not a dev build).
* Reopen. Your memory, personas, and settings are still there because they live in `~/.openhuman/`, which you didn't touch.

### Rung 3: Fix the specific error

Match your symptom:

| Symptom in logs / UI | Cause | Fix |
| -------------------- | ----- | --- |
| Sign-in stalls after the provider step; log mentions `openhuman://` scheme **not registered** (Windows) | The URL handler didn't register, or the install was moved after first launch | Follow the repair steps in [Troubleshooting Sign-In](../overview/troubleshooting-sign-in.md#windows-openhuman-handler-not-registered) |
| App won't render / crashes on launch citing a **CEF / cache lock** (`SingletonLock`, cache "held by another instance") | A previous instance's browser cache is still locked | Ensure no other OpenHuman is running; if it persists, close all instances and relaunch |
| Local AI / Ollama errors on startup | The local model runtime isn't reachable | This does **not** block the app; see [Use OpenHuman with a local model](local-model.md#common-failures) |
| "Low disk space" warning, or writes failing | The workspace can't be written | Free up space (the app wants a healthy margin, a few hundred MB minimum) and restart |

### Rung 4: Move the data folder aside (non-destructive reset)

If the app still won't start and you suspect the **data folder** itself is the problem, **rename** it rather than delete it. This gives you a clean start while keeping a full backup you can restore from.

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

Relaunch. OpenHuman recreates a fresh data folder and you sign in again.

* If the fresh start **works**, the old folder was the issue, but your data is safe in the backup. You can copy specific pieces back (your memory database and vault) and test after each.
* If it **still fails**, the data folder wasn't the cause. **Restore your backup** by renaming it back, so you lose nothing, and escalate (below).

***

## Success checks

You've recovered when:

* [ ] The app launches to the sign-in or chat screen without crashing.
* [ ] You can sign in and reach your home/chat view.
* [ ] Your **Memory** tab still shows your existing summaries (confirming your data survived).
* [ ] The most recent log file shows a clean startup with no repeating error.

## What "preserved by default" means here

At no point in Rungs 1 to 3 do you delete anything. Rung 4 **renames** your data folder; it never removes it. Even the most aggressive step is fully reversible. Reinstalling the app and reinstalling the OS-level runtime never target your `~/.openhuman/` data.

## If you're still stuck

Gather this and open an issue on [GitHub](https://github.com/tinyhumansai/openhuman) or ask on [Discord](https://discord.tinyhumans.ai):

* App version and OS.
* The last error lines from the log (**tokens/JWTs redacted**).
* Which rung you reached and what happened.
* Whether moving the data folder aside changed anything.

## See also

* [Troubleshooting Sign-In](../overview/troubleshooting-sign-in.md): the deep dive for auth-specific failures.
* [Move OpenHuman to a new PC](move-to-new-pc.md): the same data folder is what you carry over.
