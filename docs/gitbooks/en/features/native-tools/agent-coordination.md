---
description: Tools the agent uses to plan, delegate and ask for help.
icon: sitemap
---

# Agent coordination

Besides doing the work, the agent has tools for organizing it: planning multi-step jobs, spawning sub-agents, and pausing to ask you when something is unclear.

## Tools in this family

| Tool                       | What it does                                                                                                           |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `todo`                     | Rewrites the session to-do list across a long task. The chat shows it as a checklist that ticks off as work progresses. |
| `spawn_subagent`           | Starts an asynchronous child selected by `agent_id`; use `task_key` to resume a compatible child, including a paused one. |
| `use_skill`                | Loads an inline skill's playbook and tools (coding, web3, system, scheduling, docs, mcp) so the agent can call them.   |
| `ask_user_clarification`   | Pauses and asks you a precise question instead of guessing.                                                            |
| `plan_exit`                | Leaves the planning phase and starts executing.                                                                        |

`spawn_subagent` accepts an `agent_id` to choose a specialist, an optional `model` to pin that child to a model for one call, and an optional `task_key` to reuse a compatible child. If `model` is omitted, model selection uses the configured pin or normal routing hints. Calls return an asynchronous task reference; they do not wait for the child. Reusing the same `task_key` continues a compatible child and resumes a clarification-paused child when its checkpoint is available. Set `fresh: true` when the task needs a new child instead of reusing a matching session. Without a chat delivery thread, the child still runs asynchronously and its completion is recorded in the parent run log.

The returned task reference identifies the asynchronous child. Its result is delivered to the parent chat when a delivery thread is available; headless runs retain the result in the run log.

## Why these are tools

Long tasks fall apart when an agent tries to hold everything in one head. Splitting work with to-dos and sub-agents helps in three ways:

- Each sub-agent keeps useful local context for the same job instead of being respawned every turn.
- The main thread keeps a high-level view of progress.
- A failure in one branch does not spoil the rest.

Asking for clarification is a tool on purpose. It makes "I should ask the user" a visible decision the agent can be steered toward, instead of something that only happens by chance.

## See also

- [Coder](coder.md): the coding tools, most of them loaded through the `coding` skill.
- [Cron and scheduling](cron.md): how background agent runs are scheduled.
