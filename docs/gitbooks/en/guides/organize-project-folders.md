---
description: >-
  Let OpenHuman tidy, rename and restructure a folder inside a boundary you
  set, with changes held for your approval at the supervised tier.
icon: folder-tree
---

# Organize my project folders

This guide points the assistant at a folder and has it clean up: sort files, rename them consistently and remove clutter. It cannot roam your whole disk, and it cannot make changes you did not see.

The agent works inside a boundary you define. With the autonomy policy on at the `supervised` tier, any file change that is not provably read-only waits for your approval. At `full`, routine writes run on their own and only network, install and destructive actions stop to ask.

## Prerequisites

- OpenHuman set up. See [Create my personal AI assistant](personal-assistant.md).
- A specific folder you want organized. If the contents are irreplaceable, copy the folder first.

## Privacy implications

- File organizing is local. Reading, moving and renaming files happens on your machine.
- If you ask the agent to reason about file contents (for example, "group these by topic"), it may send relevant snippets to the model. Route inference to a [local model](local-model.md) if you want that reasoning on-device too.
- The agent cannot touch system or credential folders (`~/.ssh`, `~/.gnupg`, `~/.aws`, OS directories). They are blocked regardless of settings.

## Steps

### 1. Decide where the agent may act

{% hint style="warning" %}
Everything in this step needs the autonomy policy switched on. It is off by default. Set `[autonomy] enabled = true` in `config.toml` (`~/.openhuman/config.toml`, or `%USERPROFILE%\.openhuman\config.toml` on Windows). Without it, trusted roots, `workspace_only` and the approval gate do nothing, and acting tool calls run unprompted. Credential stores and system roots stay blocked either way.
{% endhint %}

Confining the agent to its working folder needs two things: the policy enabled and `workspace_only` on. With both, the agent reads and writes only in its working folder and has no access to the rest of your disk. If either is off, the boundary is not enforced.

A trusted root is a deliberate exception. It grants access to a folder outside the working folder and takes precedence over `workspace_only`. To let the agent work in a folder elsewhere, add it as a trusted root:

1. Open **Settings → Agent access**.
2. Add the target folder as a trusted root with read-write access.

Keep the boundary as tight as the task. Grant the one folder, not your home directory.

### 2. Set the autonomy tier

In the same `[autonomy]` block, `level` decides how much runs without asking:

- `supervised` (recommended): the agent proposes each change, and you approve moves, renames and deletes as they come, except for tools on the always-allow list.
- `full`: routine file writes run automatically. Still keep the trusted root tight so "automatic" stays contained.

Deleting and moving files change state, so at `supervised` the [Approval Gate](../features/approval-gate.md) holds them for your yes or no. If you answer **Always allow** to a prompt, that tool goes on the always-allow list and its calls run without a fresh approval. Remove it from the list in **Settings → Agent access** to review each call again.

### 3. Ask for the reorganization

Be concrete about the folder and the rules. For example:

- "In my trusted `~/Documents/receipts` folder, rename every file to `YYYY-MM-DD-vendor.pdf` based on its contents, and move anything older than 2023 into an `archive/` subfolder."
- "Group the loose files in this folder into subfolders by type, and show me the plan before doing anything."

### 4. Review each proposed action

When the agent wants to move, rename or delete, an Approval Request card shows the exact action. You can **Approve** it, **Always allow** a safe, repetitive one, or **Deny** it. You can also type "yes" or "no".

## Success checks

- [ ] The policy is enabled and `workspace_only` is on, so the folder boundary is enforced.
- [ ] The agent touched only the folder you granted, and nothing outside your working folder and trusted roots changed.
- [ ] Each move, rename and delete showed up as an approval prompt, unless you chose Always allow for that tool.
- [ ] The folder matches the structure you asked for.
- [ ] Files you did not mention are untouched.

## Common failures

| Symptom | Cause | Fix |
| --- | --- | --- |
| "I can't access that folder" | The folder is not a trusted root, or `workspace_only` is confining the agent | Add it in **Settings → Agent access** as a read-write trusted root |
| It asks for approval on every file | The `supervised` tier gates each write | Use **Always allow** for the specific safe tool, or narrow the request so there are fewer actions |
| It refuses to touch a path | The path is a blocked system or credential directory | This is by design. Choose a normal working folder |
| It reorganized more than you wanted | The instruction was broad | Ask it to show the plan first, and approve selectively |

## Recovery

- At the `supervised` tier nothing runs without approval, apart from tools on your always-allow list. If a plan looks wrong, **Deny** it and it does not happen.
- Undo is manual. OpenHuman does not roll back file operations, so work on a copy of anything precious, or keep the folder under version control such as `git`.
- If the agent is doing too much, set both `enabled = true` and `level = "readonly"`. The level does nothing on its own, because with `enabled = false` the whole policy is off. With both set, the agent can still suggest a plan but cannot change files.

## See also

- [Approval Gate](../features/approval-gate.md): what gets held and why.
- [Coder toolset](../features/native-tools/coder.md): the filesystem and git tools the agent uses here.
- [Privacy and security](../features/privacy-and-security.md): workspace scoping and path hardening.
