---
description: Recurring jobs, one-off reminders, and scheduled agent runs - first-class.
icon: clock
---

# Cron & Scheduling

Scheduling is a first-class capability, not a workaround. The agent can set up recurring jobs ("every weekday at 9am, summarise my inbox"), one-off reminders ("nudge me about this in three hours"), and arbitrary agent runs on a cron schedule.

## Tools in the family

| Tool          | What it does                                                       |
| ------------- | ------------------------------------------------------------------ |
| `cron_add`    | Create a new scheduled job - cron expression + agent prompt.       |
| `cron_list`   | List existing jobs and their next-run times.                       |
| `cron_update` | Edit an existing job - change schedule, prompt, or enabled state.  |
| `cron_remove` | Delete a job.                                                      |
| `cron_run`    | Run a job once, immediately, regardless of its schedule.           |
| `cron_runs`   | Inspect the recent run history - when, how long, what it produced. |

There's also a one-shot `schedule` tool in [System & Utilities](system-and-utilities.md) for "do this once at time T" cases that don't need a recurring entry.

Agent jobs run a full model turn every time they fire, so they must be scheduled at least five minutes apart. `cron_add` and `cron_update` reject a tighter cron expression or `every_ms` for an agent job and say which two runs would be too close (`*/7 * * * *`, for instance, fires at :56 and again at :00). Shell jobs have no such floor.

## What it's good for

- Daily / weekly digests delivered to your messaging channel of choice.
- Polling a slow integration that doesn't push events.
- Reminders the agent itself owns ("remind me Thursday to follow up with Alice").
- Recurring research - "every Monday, check what's new on this topic and write me a brief".

## How it ties back to the rest

Every cron run is just a normal agent invocation, so it can use any other tool - search the web, query the [Memory Tree](../obsidian-wiki/memory-tree.md), call a [third-party integration](../integrations/README.md), send a message. Run history is recorded so you can see what each tick produced.

## See also

- [System & Utilities](system-and-utilities.md) - the one-shot `schedule` tool.
- [Agent Coordination](agent-coordination.md) - for jobs that fan out into subagents.
