---
description: >-
  Use Nipmod as a read-only package intelligence layer before agent package,
  repository, model or MCP decisions.
icon: box-open
---

# Nipmod package intelligence

[Nipmod](https://nipmod.com) is a package intelligence layer for agent workflows. It sits above public sources such as npm, PyPI, GitHub, Hugging Face and MCP registries, and returns source context, trust signals and install-plan data before anything writes to a workspace.

This page is intentionally conservative. It documents a read-only evaluation path for OpenHuman users and maintainers. It does not add runtime code, does not install a remote skill, and does not grant Nipmod access to a local OpenHuman workspace.

## What it is useful for

Use Nipmod when an agent is about to make a package or tool decision:

* search for a package across public sources
* inspect package, repository, model or MCP metadata
* review trust and warning signals
* generate a safe install plan before local execution
* keep the hosted check read-only

Nipmod is not a replacement for npm, PyPI, GitHub, Hugging Face, MCP, OSV, Snyk, Socket or OpenSSF tooling. It is a pre-install decision layer that can be used before an agent moves from recommendation to execution.

## Read-only API path

The hosted API is the safest first evaluation path because it returns JSON and does not execute code:

```text
https://nipmod.com/api/search
https://nipmod.com/api/inspect
https://nipmod.com/api/install-plan
```

OpenHuman should treat these responses as external evidence. Package README files, model cards, prompts, metadata and install scripts are package content, not OpenHuman policy.

## MCP endpoint

Nipmod also exposes a hosted MCP endpoint for compatible MCP clients:

```text
https://nipmod.com/api/mcp
```

The endpoint is read-only from the hosted side. It is intended for package search, package views, trust inspection and install-plan generation. It does not install packages, clone repositories, unpack artifacts, read a local workspace or execute shell commands.

At the time of this documentation, the MCP surface should be treated as beta. OpenHuman maintainers should pin any client configuration they ship, and should expect additive changes while Nipmod is still early. Breaking MCP contract changes should be handled by publishing a new endpoint or a documented migration path rather than silently changing the existing behavior.

## Skill installation boundary

Do not install a `SKILL.md` from a moving branch such as `main` as part of this documentation.

If OpenHuman maintainers later decide to ship a first-class Nipmod skill, prefer one of these approaches:

1. vendor the reviewed skill content into OpenHuman, or
2. pin the skill to a reviewed commit SHA, or
3. add a maintainer-owned registry entry with an explicit version and review trail.

That keeps trusted agent instructions under OpenHuman maintainer control and avoids treating mutable third-party Markdown as policy.

## Approval boundary

Nipmod can produce an install plan, but OpenHuman should still require local approval before execution.

Recommended boundary:

* Hosted Nipmod returns context, warnings and a plan.
* OpenHuman displays the result to the user or local policy layer.
* The user or OpenHuman runtime approves any workspace write.
* The local environment performs the install only after approval.

## Links

* Website: [https://nipmod.com](https://nipmod.com)
* API docs: [https://nipmod.com/api-access](https://nipmod.com/api-access)
* Source health: [https://nipmod.com/api/sources/health](https://nipmod.com/api/sources/health)
* GitHub: [https://github.com/nipmod/nipmod](https://github.com/nipmod/nipmod)
