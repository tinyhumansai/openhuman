---
description: >-
  Combine OpenHuman's autonomy, approval and screening controls into a
  locked-down assistant for a child, with an honest account of the limits.
icon: child
---

# Create a safe companion for a child

This guide sets up the most restricted, supervised version of OpenHuman you can build, for a child to use with an adult present.

{% hint style="danger" %}
OpenHuman has no dedicated child mode, no age verification, and no filter on what the model says. It is not a certified parental-control product. You can combine its existing safety controls into a tightly locked-down setup. That reduces risk, but it does not make an AI assistant safe for a child to use alone. Adult supervision is the control that matters most, so do not rely on software alone.
{% endhint %}

The guide stacks the controls that exist and says plainly what each one does and does not cover.

## Prerequisites

- OpenHuman set up on the machine the child will use. See [Create my personal AI assistant](personal-assistant.md).
- An adult who owns the account and stays involved.

## Privacy implications

- Keep it local. Use a [local model](local-model.md) so conversations are not sent to a cloud provider, and connect no personal integrations.
- Do not connect a child's accounts. The safest memory is minimal memory.

## What each control does

| Control | What it protects against | What it does not do |
| --- | --- | --- |
| `readonly` autonomy (needs the policy on) | The assistant taking any action: sending, writing files, running commands or reaching the network on its own | Filter what it says |
| Approval Gate | A state-changing or network action going through without an adult's yes | Review conversation content |
| Workspace-only and blocked system directories | The agent touching files outside a small folder, or any credential or system directory | Limit which topics come up |
| Prompt-injection screening | Pasted text that tries to hijack the assistant's instructions | Moderate content in general |
| No integrations connected | The assistant pulling in or acting on personal data | Nothing else. Not connecting is the whole control |

None of these filter the model's language or subject matter. An adult in the room and persona instructions fill that gap, not a setting.

## Steps

### 1. Set the strictest autonomy tier

Go to **Settings → Agents → Agent access**:

- Set `[autonomy] enabled = true` with `level = "readonly"` in `config.toml`. The assistant can then talk and answer, but cannot act, write files or reach the network on its own. The policy is off by default, so this is the first thing to set.
- Keep `workspace_only` on.
- Leave the [Approval Gate](../features/approval-gate.md) installed as a second layer. At `readonly`, acting is blocked anyway.
- Review the auto-approve list and remove anything you do not want running without a prompt.

### 2. Keep inference and data local

- Set up a [local model](local-model.md). You run the runtime and pull the model yourself. Then route chat and reasoning to the local provider so conversations stay on-device. Adding the provider alone does not move chat. It stays on the default cloud route until you point the chat and reasoning workloads at the local provider and confirm with a test message. Turning on `local_only` [Privacy mode](../features/privacy-mode.md) makes that guarantee firm.
- Connect no integrations, and do not sign in the child's accounts.

### 3. Write a protective persona

Edit the behavior prompt (`SOUL.md`, on the **Brain** page at `/brain`) to set age-appropriate rules. For example: "You are talking with a child. Keep language simple and kind. Refuse and redirect anything violent, sexual, frightening or unsafe. Never give instructions that could cause harm. Encourage them to ask a parent." With no built-in filter, persona instructions are your main lever over content.

### 4. Supervise, and test first

- Sit with the child, at least at first.
- Before handing it over, try to break it yourself. Ask it things a child might, and confirm the persona redirects well.

## Success checks

- [ ] `[autonomy] enabled = true`, `level = "readonly"`, and `workspace_only` is on.
- [ ] No integrations are connected.
- [ ] Inference is local (`ready`), so conversations do not go to a cloud provider.
- [ ] In your own testing, the persona refuses and redirects unsafe prompts.
- [ ] An adult is present during use. This is a check, not a nicety.

## Common failures

| Symptom | Cause | Fix |
| --- | --- | --- |
| It produced content you consider inappropriate | There is no content filter, and the persona alone governs tone | Strengthen the `SOUL.md` rules and supervise. This is a limit of the tool |
| It tried to send, open or fetch something | The policy is off, or the tier is not `readonly` | Set `enabled = true` and `level = "readonly"` in `config.toml` |
| Conversation went to the cloud | Chat is not routed to a local provider | Route chat to a [local model](local-model.md) and send a test message |
| The child reached settings and changed things | OpenHuman has no separate child login | Use OS-level user accounts or parental controls to lock down the machine |

## Recovery

- **Lock down at once.** Set `enabled = true` and `level = "readonly"`, if either drifted. The level does nothing while `enabled` is `false`, so check both. Acting stops on the next turn.
- **Reset the persona.** Revert your `SOUL.md` edits to the defaults if the customization misbehaves.
- **Supervise.** If the experience is not right for the child, step in. No setting replaces that.

## See also

- [Keep sensitive data private](privacy-sensitive-data.md): the controls this guide stacks.
- [Approval Gate](../features/approval-gate.md): the autonomy tiers and what each one blocks.
- [Privacy and security](../features/privacy-and-security.md): what leaves the machine, and what does not.
