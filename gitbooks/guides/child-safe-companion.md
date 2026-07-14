---
description: >-
  Compose OpenHuman's autonomy, approval, and content-screening controls into a
  locked-down assistant for a child, with an honest account of the limits.
icon: child
---

# Create a safe companion for a child

**Goal:** set up the most restricted, supervised version of OpenHuman you can, intended for a child to use with an adult present.

{% hint style="danger" %}
**Read this first. It is an honest warning.** OpenHuman has **no dedicated "child mode"**, no age verification, and no content filter on what the model *says*. There is no certified parental-control product here. What you *can* do is compose the existing safety controls into a tightly locked-down setup. That reduces risk; it does not make an AI assistant a safe, unsupervised experience for a child. **Adult supervision is the control that matters most.** Do not rely on software alone.
{% endhint %}

This guide is about stacking the real controls that exist, and being clear about what they do and don't cover.

***

## Prerequisites

* OpenHuman set up on the machine the child will use. See [Create my personal AI assistant](personal-assistant.md).
* An adult who owns the account and stays involved.

## Privacy implications

* Keep it local. Use a [local model](local-model.md) so conversations aren't sent to a cloud provider, and connect **no** personal integrations.
* Do not connect a child's accounts. The safest memory is minimal memory.

## What each control actually does

| Control | What it protects against | What it does **not** do |
| ------- | ------------------------ | ----------------------- |
| **Read-only autonomy** | The assistant taking any action (sending, writing files, running commands, or reaching the network on its own) | Doesn't filter what it *says* |
| **Approval Gate (on)** | Any state-changing/network action slipping through without an adult's yes | Doesn't review conversation content |
| **Workspace-only + blocked system dirs** | The agent touching files outside a small folder, or any credential/system directory | Doesn't restrict what topics come up |
| **Prompt-injection screening** | Attempts (in pasted text) to hijack the assistant's instructions | Isn't a general content moderator |
| **No integrations connected** | The assistant pulling in or acting on personal data | (Nothing; not connecting is the whole control) |

The honest gap: **none of these filter the model's language or subject matter.** That gap is filled by an adult in the room and by persona instructions, not by a setting.

***

## Steps

### 1. Set the strictest autonomy tier

**Settings → Agents → Agent access:**

* Autonomy: **Read-only**. The assistant can talk and answer, but cannot act, write files, or reach the network on its own.
* Keep **workspace-only** on.
* Keep the [Approval Gate](../features/approval-gate.md) on. (With Read-only, acting is blocked outright anyway. Leave the gate on as a second layer.)
* Review the **auto-approve** list and remove anything you don't want running without a prompt.

### 2. Keep inference and data local

* Turn on a [local model](local-model.md), then **route chat and reasoning at the local provider** so conversations run on-device. Enabling Local AI alone keeps only embeddings/memory local. Chat still uses the default cloud route until you point the chat/reasoning workloads at the local provider and confirm with a test message.
* Connect **no** integrations. Don't sign the child's accounts in.

### 3. Write a protective persona

Edit the behavior prompt (`SOUL.md`, via the **Brain** page `/brain`) to set age-appropriate rules directly. For example: "You are talking with a child. Keep language simple and kind. Refuse and redirect anything violent, sexual, frightening, or unsafe. Never give instructions that could cause harm. Encourage them to ask a parent." Persona instructions are your main lever over *content*, since there's no built-in filter.

### 4. Supervise, and test first

* Sit with the child, at least at first.
* Before handing it over, **try to break it yourself**: ask it things a child might, and confirm the persona redirects appropriately.

***

## Success checks

* [ ] Autonomy is **Read-only**; workspace-only and the approval gate are on.
* [ ] No integrations are connected.
* [ ] Inference is local (`ready`), so conversations aren't going to a cloud provider.
* [ ] In your own testing, the persona refuses and redirects unsafe prompts.
* [ ] An adult is present for use. *(This is a check, not a nicety.)*

## Common failures

| Symptom | Cause | Fix |
| ------- | ----- | --- |
| It produced content you consider inappropriate | There is no content filter; the persona alone governs tone | Strengthen the `SOUL.md` rules; supervise; this is an inherent limit of the tool |
| It tried to do something (send/open/fetch) | Tier isn't Read-only | Set autonomy to **Read-only** in Agent access |
| Conversation went to the cloud | Local AI isn't on | Turn on a [local model](local-model.md) and confirm `ready` |
| The child reached settings and changed things | OpenHuman has no separate child login | Use OS-level user accounts/parental controls to lock down the machine itself |

## Recovery

* **Instant lockdown:** set autonomy to **Read-only** (if it drifted). Acting stops next turn.
* **Reset persona:** revert your `SOUL.md` edits to defaults if the customization misbehaves.
* **The real recovery is supervision.** If the experience isn't right for the child, step in. No software setting substitutes for that.

## See also

* [Keep sensitive data private](privacy-sensitive-data.md): the controls this guide stacks.
* [Approval Gate](../features/approval-gate.md): how actions are gated.
* [Privacy & Security](../features/privacy-and-security.md): autonomy tiers and path hardening in depth.
