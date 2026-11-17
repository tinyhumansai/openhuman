---
description: >-
  Step-by-step guides for getting a result out of OpenHuman: set up an
  assistant, run local models, protect sensitive data, recover a broken install
  or move to a new machine.
icon: list-check
---

# Guides

The rest of the docs explain how OpenHuman works. These guides are about what you want to get done. Each one starts from a concrete goal, such as "I want a private assistant", "my install is broken" or "I'm moving to a new laptop", and walks you to a working result. You do not need to read them in order or be technical.

## Pick your goal

| I want to | Guide |
| --- | --- |
| Set up a personal assistant from scratch | [Create my personal AI assistant](personal-assistant.md) |
| Keep model inference on my own machine | [Use OpenHuman with a local model](local-model.md) |
| Spread work across several model servers I run | [Use multiple local LLM servers](multiple-local-llm-servers.md) |
| Understand what leaves my computer and what doesn't | [Keep sensitive data private](privacy-sensitive-data.md) |
| Fix an install that won't start or finish | [Recover from a failed installation](recover-failed-installation.md) |
| Move everything to a new computer | [Move OpenHuman to a new PC](move-to-new-pc.md) |
| Let the agent tidy a folder of files | [Organize my project folders](organize-project-folders.md) |
| Build a role-specific assistant, such as a clinical one | [Create a doctor-specific assistant](doctor-assistant.md) |
| Set up a locked-down assistant for a child | [Create a safe companion for a child](child-safe-companion.md) |

## How a guide is laid out

Every guide has the same sections:

- **Prerequisites**: what you need before you start.
- **Privacy implications**: what stays on your machine and what goes to the OpenHuman backend or a model provider.
- **Steps**: the path to follow.
- **Success checks**: how to confirm it works.
- **Common failures**: how the flow breaks, and what each symptom means.
- **Recovery**: how to get unstuck without losing data.

{% hint style="info" %}
These guides describe the shipping desktop app, which is in active development. If a screen name and what you see disagree, trust the app and check the [release notes](https://github.com/tinyhumansai/openhuman/releases). Then tell us on [Discord](https://guild.tinyhumans.ai).
{% endhint %}

## Where your data lives

OpenHuman keeps your settings, workspace files and secrets on your machine. Your memory is stored in CortexDB, either the hosted one TinyHumans runs behind your account or your own. The managed backend also handles sign-in, model routing, integration access, web-search proxying and some real-time integration triggers. That split is behind most of the privacy and recovery advice here. If you read one background page, read [Privacy and security](../features/privacy-and-security.md).

On disk, everything lives in one folder:

| Platform | Data folder |
| --- | --- |
| macOS and Linux | `~/.openhuman/` |
| Windows | `%USERPROFILE%\.openhuman\` |

Backups, recovery and migration all come back to that folder. Memory items are not in it, so backing up or deleting the folder neither copies nor erases them.

## See also

- [Privacy and security](../features/privacy-and-security.md): the trust model these guides assume.
- [Local AI](../features/model-routing/local-ai.md): the config reference behind the local-model guides.
- [Memory](../features/memory.md): what memory stores and where.
- [Getting started](../overview/getting-started.md): install and first run.
- [Troubleshooting sign-in](../overview/troubleshooting-sign-in.md): if you cannot get past login.
- [Developing](../developing/README.md): building from source.
