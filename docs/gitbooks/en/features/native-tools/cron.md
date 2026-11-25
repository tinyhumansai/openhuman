---
description: Recurring jobs, one-off reminders and scheduled agent runs.
icon: clock
---

# Cron and scheduling

Scheduling is a built-in capability. The agent can set up recurring jobs ("every weekday at 9am, summarize my inbox"), one-off reminders ("nudge me about this in three hours") and any agent run on a cron schedule.

## Tools in this family

| Tool          | What it does                                                          |
| ------------- | --------------------------------------------------------------------- |
| `cron_add`    | Creates a scheduled job from a cron expression and an agent prompt.   |
| `cron_list`   | Lists existing jobs and their next run times.                         |
| `cron_update` | Edits a job: its schedule, prompt or enabled state.                   |
| `cron_remove` | Deletes a job.                                                        |
| `cron_run`    | Runs a job once, right now, whatever its schedule.                    |
| `cron_runs`   | Shows recent run history: when, how long and what it produced.        |

For one-shot or recurring scheduled tasks, there is also a `schedule` tool in [System and utilities](system-and-utilities.md).

Agent jobs run a full model turn each time they fire, so they must be at least five minutes apart. `cron_add` and `cron_update` reject a tighter cron expression or `every_ms` for an agent job, and they say which two runs would be too close. For example, `*/7 * * * *` fires at :56 and again at :00. Shell jobs have no such limit.

## What it is good for

- Daily or weekly digests delivered to your messaging channel.
- Polling a slow integration that doesn't push events.
- Reminders the agent owns, such as "remind me Thursday to follow up with Alice".
- Recurring research, such as "every Monday, check what's new on this topic and write me a brief".

## How it fits with the rest

A cron run is a normal agent run, so it can use any other tool: search the web, query [memory](../memory.md), call a [third-party integration](../integrations/README.md), or send a message. Run history is recorded so you can see what each run produced.

## See also

- [System and utilities](system-and-utilities.md): the one-shot `schedule` tool.
- [Agent coordination](agent-coordination.md): for jobs that fan out into sub-agents.
