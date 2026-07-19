---
description: >-
  Outcome-driven, step-by-step guides for getting a real result out of
  OpenHuman: set up an assistant, run a local model, protect sensitive data,
  recover a broken install, or move to a new machine.
icon: list-check
---

# Guides

The rest of the docs explain **how OpenHuman works**. This section is about **what you want to get done**. Each guide starts from a concrete outcome ("I want a private assistant", "my install is broken", "I'm moving to a new laptop") and walks you to a working result without asking you to first understand the architecture.

You do not need to read these in order, and you do not need to be technical. Pick the guide that matches your goal.

***

## Pick your goal

| I want to… | Guide |
| ---------- | ----- |
| Set up a personal assistant from scratch | [Create my personal AI assistant](personal-assistant.md) |
| Keep model inference on my own machine | [Use OpenHuman with a local model](local-model.md) |
| Understand what leaves my computer and what doesn't | [Keep sensitive data private](privacy-sensitive-data.md) |
| Fix an install that won't start or finish | [Recover from a failed installation](recover-failed-installation.md) |
| Move everything to a new computer | [Move OpenHuman to a new PC](move-to-new-pc.md) |
| Read and edit memory in Obsidian | [Connect OpenHuman to Obsidian](connect-obsidian.md) |
| Let the agent tidy a folder of files | [Organize my project folders](organize-project-folders.md) |
| Build a role-specific assistant (e.g. clinical) | [Create a doctor-specific assistant](doctor-assistant.md) |
| Set up a locked-down assistant for a child | [Create a safe companion for a child](child-safe-companion.md) |

***

## How to read a guide

Every guide in this section follows the same shape, so you always know where to look:

* **Prerequisites**: what you need before you start (an account, disk space, another app installed).
* **Privacy implications**: what stays on your machine and what is sent to the OpenHuman backend or a model provider for this workflow, in plain language.
* **Steps**: the actual click-by-click path.
* **Success checks**: how to confirm the workflow is actually working, not just "looks done".
* **Common failures**: the specific ways this flow breaks, and what each symptom means.
* **Recovery**: how to get unstuck without losing your data.

{% hint style="info" %}
**These guides describe the shipping desktop app.** OpenHuman is in active development; where a guide points at a Settings screen or diagnostic surface, that surface may keep improving. When a screen name and what you see disagree, trust the app and check the [release notes](https://github.com/tinyhumansai/openhuman/releases), then let us know on [Discord](https://discord.tinyhumans.ai).
{% endhint %}

## The one thing worth knowing first

OpenHuman keeps **the memory of your life on your machine**. The managed backend still brokers sign-in, model routing, integration access, web-search proxying, and some real-time integration triggers. That single fact is behind most of the privacy and recovery advice in this section. If you read only one background page, read [Privacy & Security](../features/privacy-and-security.md).

Where your data physically lives on disk:

| Platform | Data folder |
| -------- | ----------- |
| macOS / Linux | `~/.openhuman/` |
| Windows | `%USERPROFILE%\.openhuman\` |

Almost everything in this section (backups, recovery, migration) comes back to that one folder.
